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
