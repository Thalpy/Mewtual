use super::*;
use crate::{studio::StudioSettlementState as S, AppEvent};

#[tokio::test]
async fn studio_settlement_events_follow_writes_not_refresh_reads_and_do_not_hold_custody() {
    let mut p = Pair::new().await;
    let logical = target().document(&p.alice.group_id()).unwrap();
    let created = p
        .alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Create {
                channel: channel(),
                object: [7; 16],
                nonce: [1; 16],
                title: "moon".into(),
                ts: 1000,
            },
        )
        .unwrap()
        .unwrap();
    let mut ids = Vec::new();
    for n in 1..=3 {
        let s = snapshot(&created.projection, n);
        ids.push(s.id().unwrap());
        p.a_store
            .update_epoch_recovery(
                SERVER,
                &logical,
                crate::store::EpochRecoveryAction::Stage(s),
                &ManualClock::new(100),
                &mut rng(),
            )
            .unwrap();
    }
    let store = Arc::new(Mutex::new(Some(p.a_store)));
    let (actor, mut events, task) = crate::spawn(p.alice);
    // A worker reply precedes the event send. A subsequent actor command provides an exact
    // scheduling barrier, avoiding sleeps and races with an asynchronous drain task.
    invoke(&actor, &store, Action::List).await.unwrap();
    assert_eq!(actor.member_count().await, 2);
    while let Ok(ev) = events.try_recv() {
        assert!(!matches!(ev.event, AppEvent::SettlementChanged { .. }));
    }
    invoke(
        &actor,
        &store,
        Action::Acknowledge {
            oldest_snapshot: ids[0],
            staged_snapshot: ids[2],
        },
    )
    .await
    .unwrap();
    assert_eq!(actor.member_count().await, 2);
    assert!(
        store.try_lock().is_ok(),
        "no event await may retain the vault lease"
    );
    let mut states = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let AppEvent::SettlementChanged { target: t, state } = ev.event {
            assert_eq!(t, target());
            states.push(state);
        }
    }
    assert_eq!(
        states,
        vec![S::RefreshRequired, S::Open, S::RecoveryAvailable]
    );
    // A rejected stale warning requests a refresh, never invents a Fault/Closing/disk-full state.
    assert!(invoke(
        &actor,
        &store,
        Action::Acknowledge {
            oldest_snapshot: [99; 32],
            staged_snapshot: ids[2]
        }
    )
    .await
    .is_err());
    assert_eq!(actor.member_count().await, 2);
    let mut states = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let AppEvent::SettlementChanged { state, .. } = ev.event {
            states.push(state);
        }
    }
    assert_eq!(states, vec![S::RefreshRequired]);
    invoke(&actor, &store, Action::List).await.unwrap();
    assert_eq!(actor.member_count().await, 2);
    while let Ok(ev) = events.try_recv() {
        assert!(!matches!(ev.event, AppEvent::SettlementChanged { .. }));
    }
    actor.shutdown().await;
    task.await.unwrap();
}

/// The destructive lifecycle actions request a refresh, and the read-only ones do not.
///
/// Membership of the receiver's `changing` list is the whole guard, and it had none: a review found
/// that nothing named `DisposeOverlay`, `ReleaseOverlayArchive` or `FinishOverlayArchive` as
/// emitters, so the list could have lost any of them silently. A renderer that got no refresh after
/// a release would keep showing an archive that is gone.
///
/// **Every action here FAILS, and that is the point.** The notice follows the action, not its
/// outcome, because the case that matters is the one where a write may already have landed - a
/// release whose unlink succeeded and whose parent sync did not. An implementation that emitted
/// only on success would be silent exactly when the renderer is most wrong.
///
/// The read-only half is not decoration: without it, a receiver that emitted for everything would
/// satisfy the first half and be just as broken.
#[tokio::test]
async fn destructive_overlay_actions_request_a_refresh_even_when_they_refuse() {
    use crate::store::{StudioDisposalRequestMode, StudioOverlayDisposalRequest};
    use crate::studio::{StudioArchiveReleaseRequest, StudioReleaseConfirmation};

    let p = Pair::new().await;
    let store = Arc::new(Mutex::new(Some(p.a_store)));
    let (actor, mut events, task) = crate::spawn(p.alice);

    fn drain(events: &mut tokio::sync::mpsc::Receiver<crate::actor::TracedEvent>) -> Vec<S> {
        let mut states = Vec::new();
        while let Ok(ev) = events.try_recv() {
            if let AppEvent::SettlementChanged { state, .. } = ev.event {
                states.push(state);
            }
        }
        states
    }

    // This vault has no branch and no archive, so all three refuse. None of them may be silent.
    for (name, action) in [
        (
            "dispose",
            Action::DisposeOverlay(Box::new(StudioOverlayDisposalRequest {
                branch: [1; 32],
                content: [2; 32],
                accepted: 1,
                mode: StudioDisposalRequestMode::Preserve,
            })),
        ),
        (
            "release",
            Action::ReleaseOverlayArchive(Box::new(StudioArchiveReleaseRequest {
                archive: [3; 32],
                confirmation: StudioReleaseConfirmation::parse(StudioReleaseConfirmation::TOKEN)
                    .unwrap(),
            })),
        ),
    ] {
        assert!(
            invoke(&actor, &store, action).await.is_err(),
            "{name} must refuse on an empty vault, or this test is measuring something else"
        );
        assert_eq!(actor.member_count().await, 2);
        assert_eq!(
            drain(&mut events),
            vec![S::RefreshRequired],
            "{name} must request a refresh even when it refuses"
        );
    }

    // And the read-only members of the same family stay silent, refusal or not.
    invoke(&actor, &store, Action::OverlayLifecycle).await.unwrap();
    assert_eq!(actor.member_count().await, 2);
    assert!(
        drain(&mut events).is_empty(),
        "classifying a draft changes nothing and must not ask for a refresh"
    );
    assert!(invoke(&actor, &store, Action::ReadOverlayArchive)
        .await
        .is_err());
    assert_eq!(actor.member_count().await, 2);
    assert!(
        drain(&mut events).is_empty(),
        "a failed read changes nothing either"
    );

    actor.shutdown().await;
    task.await.unwrap();
}
