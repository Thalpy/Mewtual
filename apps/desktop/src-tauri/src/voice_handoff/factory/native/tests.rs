use super::super::tests::{channel_route, park, selection};
use super::*;
use crate::voice_activation_tests::Fixture;
use std::future::{poll_fn, Future};
use std::task::Poll;

fn worker(context: &Context, route: &Route, selector: PairSelector) -> Worker {
    let selected = route.selected();
    let credit = selected
        .resources()
        .charge(
            selected.binding().scope.group,
            NATIVE_TASK_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    Worker {
        context: Some(context.clone()),
        selector,
        custody: NativeCustody {
            _credit: credit,
            route: route.clone(),
            _native: route.session().native_hold().unwrap(),
        },
    }
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) -> bool {
    let mut future = future;
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
}

#[tokio::test]
async fn native_preflight_and_uncollected_output_keep_the_original_payment_and_shutdown_owner() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let pool = selected.resources();
    let before = pool.counts();
    let parked = park(&f.actor).await;
    let mut future = Box::pin(run_native(worker(&context, &route, selector)));
    let bytes = layout(future.as_ref().get_ref());
    assert!(
        bytes <= NATIVE_TASK_BYTES,
        "whole native factory requires {bytes}"
    );
    let store_owners = Arc::strong_count(&f.state.store);
    assert!(pending(future.as_mut()).await);
    assert_eq!(Arc::strong_count(&f.state.store), store_owners);
    assert_eq!(pool.counts(), (before.0 + NATIVE_TASK_BYTES, before.1));
    let physical = route.session().header().unwrap();
    drop(physical); // actual physical traffic and native preparation own different capacity
    let mut shutdown = Box::pin(f.state.voice_channels.stop_and_wait_for_physical());
    assert!(pending(shutdown.as_mut()).await);
    assert!(matches!(
        route.session().native_hold(),
        Err(super::super::super::channel::Refused::Binding)
    ));
    drop(parked);
    let output = future.await;
    assert_eq!(output.result(), Err(Refused::Closed));
    assert!(pending(shutdown.as_mut()).await);
    assert_eq!(pool.counts(), (before.0 + NATIVE_TASK_BYTES, before.1));
    drop(output);
    shutdown.await;
    assert_eq!(pool.counts(), before);
    f.close().await;
}

#[tokio::test]
async fn native_factory_ui_wait_does_not_revive_after_a_lock_unlock_cycle() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let boundary = f.state.ui_session_commit.lock().await;
    let mut future = Box::pin(run_native(worker(&context, &route, selector)));
    assert!(pending(future.as_mut()).await);
    f.actor.member_count().await; // original binding query has actually completed
    assert!(pending(future.as_mut()).await);
    crate::register_ui_lock_request(&f.state);
    drop(boundary);
    let output = future.await;
    assert_eq!(output.result(), Err(Refused::Closed));
    assert!(f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    drop(output);
    crate::lock_session_inner(&f.state, None).await.unwrap();
    let generation = f
        .state
        .ui_session_generation
        .load(std::sync::atomic::Ordering::Acquire);
    crate::finalize_unlock_session(&f.state, generation, false)
        .await
        .unwrap();
    assert!(matches!(
        produce_derivation(&context, &route, selector),
        Err(Refused::Closed)
    ));
    assert!(f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    f.close().await;
}

#[tokio::test]
async fn losing_the_native_factory_waiter_cannot_let_shutdown_pass_an_admitted_actor_turn() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let boundary = f.state.ui_session_commit.lock().await;
    let mut future = Box::pin(run_native(worker(&context, &route, selector)));
    assert!(pending(future.as_mut()).await);
    f.actor.member_count().await;
    assert!(pending(future.as_mut()).await);
    let parked = park(&f.actor).await;
    drop(boundary);
    assert!(pending(future.as_mut()).await);
    assert!(f.state.ui_session_commit.try_lock().is_err());
    let waiter = NativeWaiter {
        task: tokio::spawn(future),
    };
    drop(waiter); // the opaque observer detaches; it exposes no abort capability
    let mut shutdown = Box::pin(f.state.voice_channels.stop_and_wait_for_physical());
    assert!(pending(shutdown.as_mut()).await);
    assert!(f.state.ui_session_commit.try_lock().is_err());
    drop(parked);
    tokio::time::timeout(std::time::Duration::from_secs(15), shutdown)
        .await
        .unwrap();
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    assert!(f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_some());
    f.close().await;
}

#[tokio::test]
async fn native_producer_refuses_a_foreign_route_before_ledger_or_actor_admission() {
    let f = Fixture::new().await;
    let other = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, _) = selection(&f).await;
    let (_, foreign, selector) = selection(&other).await;
    let route = channel_route(&other, &foreign).await;
    assert!(matches!(
        produce_derivation(&context, &route, selector),
        Err(Refused::Closed)
    ));
    assert!(f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    assert!(other
        .state
        .voice_handoff
        .lookup_original(&foreign, Purpose::Derive(selector))
        .is_none());
    assert!(route.session().native_hold().is_ok());
    f.close().await;
    other.close().await;
}

#[tokio::test]
async fn the_same_selected_allocation_in_a_second_registry_is_not_canonical_native_custody() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let original = channel_route(&f, &selected).await;
    let foreign = super::super::super::bootstrap::Registry::default();
    let (_public, route) =
        super::super::super::bootstrap::fixture_installed(&foreign, selected.clone());
    assert!(Arc::ptr_eq(original.selected(), route.selected()));
    assert!(route.require_open().is_ok());
    let before = selected.resources().counts();
    let parked = park(&f.actor).await;
    assert!(matches!(
        produce_derivation(&context, &route, selector),
        Err(Refused::Closed)
    ));
    assert!(f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    assert_eq!(selected.resources().counts(), before);
    assert!(original.session().header().is_ok());
    assert!(original.session().native_hold().is_ok());
    assert!(route.session().native_hold().is_ok());
    drop(parked);
    f.close().await;
}

#[tokio::test]
async fn native_payment_refusal_precedes_preflight_and_does_not_claim_a_session_or_row() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let pool = selected.resources();
    let group = selected.binding().scope.group;
    let pressure = pool
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.group_counts(group).unwrap().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    let parked = park(&f.actor).await;
    assert!(matches!(
        produce_derivation(&context, &route, selector),
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    assert_eq!(pool.counts(), before);
    assert!(route.session().native_hold().is_ok());
    assert!(f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    drop(pressure);
    drop(parked);
    let output = produce_derivation(&context, &route, selector)
        .unwrap()
        .await
        .unwrap();
    // The synthetic fixture has no proven remote pair; real blocked-TCP tests require
    // actual successful production. Here only the intended Actor refusal is acceptable.
    assert_eq!(
        output.result(),
        Err(Refused::Protocol(VoiceError::Unauthorized))
    );
    drop(output);
    f.close().await;
}
