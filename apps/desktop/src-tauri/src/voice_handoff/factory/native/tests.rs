use super::super::tests::{channel_route, park, selection};
use super::*;
use crate::voice_activation_tests::Fixture;
use std::future::{poll_fn, Future};
use std::task::Poll;

fn worker(context: &Context, route: &Route, selector: PairSelector) -> Worker {
    selected_worker(context, route, Selection::Exact(selector))
}

fn selected_worker(context: &Context, route: &Route, selector: Selection) -> Worker {
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
        discovery_refusal: None,
        fixture_late_contention: false,
        fixture_witness: None,
        custody: NativeCustody {
            _credit: credit,
            route: route.clone(),
            _native: route.session().native_hold().unwrap(),
        },
    }
}

/// Run inside the real two-peer integration fixture's original observer bound and
/// deferred first cadence. No selector or positive Root state is fabricated here.
/// Actual discovery and its Actor preflight must pass before the real UI wait. A
/// worker-local negative-only seam holds Index AFTER the blocking known lookup and
/// immediately before its genuine nonblocking final admission check. The later
/// Capacity MUST NOT become discovery-retry evidence.
pub(in crate::voice_handoff) async fn prove_discovery_late_refusal(
    context: &Context,
    route: &Route,
    clock: &dyn catcoms_rt::Clock,
) {
    let live = context.upgrade().unwrap();
    let actor = {
        let servers = live.servers.lock().await;
        let binding = route.selected().binding();
        servers.get(&binding.server).unwrap().actor.clone()
    };
    loop {
        let boundary = live.commit.clone().lock_owned().await;
        let permit = PeerSnapshotPermit::new(
            &route.selected().resources(),
            route.selected().binding().scope,
        )
        .unwrap();
        let mut worker = selected_worker(context, route, Selection::Discover { cursor: 0, permit });
        worker.fixture_late_contention = true;
        let mut producing = Box::pin(run_native(worker));
        let mut refused = false;
        for _ in 0..3 {
            let poll = poll_fn(|cx| Poll::Ready(producing.as_mut().poll(cx))).await;
            match poll {
                Poll::Ready(completion) => {
                    assert!(
                        completion.refused_before_selection(0),
                        "negative control failed before reaching UI: {:?}",
                        completion.result()
                    );
                    drop(completion);
                    refused = true;
                    break;
                }
                Poll::Pending => {
                    // The first FIFO joins the genuine peer snapshot, the second
                    // joins preflight. Only the held original UI guard can park the
                    // third poll: there are exactly those two Actor awaits before it.
                    actor.member_count().await;
                }
            }
        }
        if refused {
            drop(boundary);
            clock.sleep(std::time::Duration::from_millis(10)).await;
            continue;
        }
        drop(boundary);
        let completion = producing.await;
        assert_eq!(
            completion.result(),
            Err(Refused::Protocol(VoiceError::Capacity))
        );
        assert!(
            !completion.refused_before_selection(0),
            "a genuine discovery followed by late Capacity is not retryable discovery work"
        );
        drop(completion);
        drop(
            route
                .session()
                .native_hold()
                .expect("actual native custody returned"),
        );
        loop {
            match live.ledger.automatic_parent_available(route.selected()) {
                Ok(true) => break,
                Err(VoiceError::Capacity) => {
                    clock.sleep(std::time::Duration::from_millis(10)).await
                }
                result => panic!("late pre-Begin refusal created a parent: {result:?}"),
            }
        }
        return;
    }
}

#[tokio::test]
async fn lost_confirmation_observer_keeps_parked_snapshot_paid_and_blocks_actual_shutdown() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, _) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let grant = selected.grant().unwrap();
    let owners = Arc::strong_count(&grant);
    let pool = selected.resources();
    let before = pool.counts();
    let parked = park(&f.actor).await;
    let observer = produce_confirmation(&context, &route, 0, true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while Arc::strong_count(&grant) <= owners {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual original snapshot command must reach the parked Actor");
    assert_eq!(
        pool.counts(),
        (
            before.0 + NATIVE_TASK_BYTES + CryptoActionSnapshotPermit::MEMORY_BYTES,
            before.1
        )
    );
    drop(observer); // observing caller loss must detach, not cancel the actual RPC/task
    let mut shutdown = Box::pin(f.state.voice_channels.stop_and_wait_for_physical());
    assert!(pending(shutdown.as_mut()).await);
    assert!(route.session().native_hold().is_err());
    assert_eq!(
        pool.counts(),
        (
            before.0 + NATIVE_TASK_BYTES + CryptoActionSnapshotPermit::MEMORY_BYTES,
            before.1
        )
    );
    drop(parked);
    tokio::time::timeout(std::time::Duration::from_secs(10), shutdown)
        .await
        .unwrap();
    assert_eq!(pool.counts(), before);
    drop(grant);
    f.close().await;
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) -> bool {
    let mut future = future;
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
}

#[tokio::test]
async fn automatic_parent_admission_rechecks_after_the_actual_ui_wait() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    assert!(f
        .state
        .voice_handoff
        .automatic_parent_available(&selected)
        .unwrap());

    // Discovery's available observation cannot authorize publication after a wait.
    // Park the actual producer after its Actor preflight, at the shared UI commit.
    let boundary = f.state.ui_session_commit.lock().await;
    let mut producing = Box::pin(run_native(worker(&context, &route, selector)));
    assert!(pending(producing.as_mut()).await);
    f.actor.member_count().await;
    assert!(pending(producing.as_mut()).await);

    let mut competing = selector;
    competing.join[0] ^= 1;
    let Begin::Factory(registration) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(competing))
        .unwrap()
    else {
        panic!("new competing original parent")
    };
    registration.mark_handoff().unwrap();
    drop(registration); // Ambiguous original handoff remains counted and recoverable.
    drop(boundary);

    let completion = producing.await;
    assert_eq!(
        completion.result(),
        Err(Refused::Protocol(VoiceError::Capacity))
    );
    assert!(
        !completion.refused_before_selection(0),
        "a refusal after selection must not be retried as discovery-only work"
    );
    assert!(
        f.state
            .voice_handoff
            .lookup_original(&selected, Purpose::Derive(selector))
            .is_none(),
        "the stale observation must not create a second Pending or call Root"
    );
    drop(completion);
    f.state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(competing))
        .unwrap()
        .close_pending()
        .unwrap();
    f.close().await;
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
async fn native_factory_waiting_for_ui_cannot_cross_the_close_first_effect_fence() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let Begin::Factory(registration) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("original unseen Pending");
    };
    registration.mark_handoff().unwrap();
    let boundary = f.state.ui_session_commit.lock().await;
    let mut closing = Box::pin(crate::voice_handoff::bootstrap::settle_setup(
        &f.state,
        selected.id(),
        selected.executor(),
    ));
    assert!(pending(closing.as_mut()).await); // actual close queues before final factory admission
    let mut producing = Box::pin(run_native(worker(&context, &route, selector)));
    assert!(pending(producing.as_mut()).await);
    f.actor.member_count().await;
    assert!(pending(producing.as_mut()).await);
    drop(boundary);
    assert!(matches!(
        closing.await,
        Err(crate::voice_handoff::channel::Refused::Capacity)
    ));
    let completion = producing.await;
    assert_eq!(completion.result(), Err(Refused::Closed));
    assert!(selected.lease().is_live());
    assert!(f.state.voice_channels.require_cleanup_route(&route).is_ok());
    drop(completion);
    drop(registration);
    let row = f
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .unwrap();
    assert!(
        row.status().unwrap().action.is_none(),
        "queued producer never published after the fence"
    );
    row.close_pending().unwrap();
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
