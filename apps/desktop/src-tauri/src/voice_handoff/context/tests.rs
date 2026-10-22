use super::*;
use crate::voice_activation_tests::Fixture;
use crate::{register_ui_lock_request, require_unlocked_session, voice_runtime};
use std::future::{poll_fn, Future};
use std::task::Poll;

fn strong_counts(state: &AppState) -> [usize; 9] {
    [
        Arc::strong_count(&state.servers),
        Arc::strong_count(&state.store),
        Arc::strong_count(&state.session_resumable),
        Arc::strong_count(&state.session_lock_requested),
        Arc::strong_count(&state.ui_session_generation),
        Arc::strong_count(&state.ui_authorized_generation),
        Arc::strong_count(&state.ui_session_commit),
        Arc::strong_count(&state.voice_authority),
        Arc::strong_count(&state.voice_handoff),
    ]
}

#[test]
fn weak_context_never_retains_native_authority_or_the_mounted_state() {
    let state = AppState::default();
    let before = strong_counts(&state);
    let context = Context::new(&state);
    assert_eq!(strong_counts(&state), before);
    let servers = context.servers.clone();
    let store = context.store.clone();
    let authority = context.authority.clone();
    let ledger = context.ledger.clone();
    let pool = voice_runtime(&state).resources();
    drop(state);
    assert!(context.upgrade().is_none());
    assert!(servers.upgrade().is_none() && store.upgrade().is_none());
    assert!(authority.upgrade().is_none() && ledger.upgrade().is_none());
    assert_eq!(pool.counts().0, 0);
}

#[test]
fn each_missing_original_owner_refuses_and_releases_all_partial_upgrades() {
    let state = AppState::default();
    let before = strong_counts(&state);
    let pool = voice_runtime(&state).resources();
    let counts = pool.counts();
    for field in 0..9 {
        let mut context = Context::new(&state);
        match field {
            0 => context.servers = Weak::new(),
            1 => context.store = Weak::new(),
            2 => context.resumable = Weak::new(),
            3 => context.requested = Weak::new(),
            4 => context.generation = Weak::new(),
            5 => context.authorized = Weak::new(),
            6 => context.commit = Weak::new(),
            7 => context.authority = Weak::new(),
            8 => context.ledger = Weak::new(),
            _ => unreachable!(),
        }
        assert!(context.upgrade().is_none(), "missing field {field}");
        assert_eq!(
            strong_counts(&state),
            before,
            "partial upgrade leaked field {field}"
        );
        assert_eq!(pool.counts(), counts, "refusal admitted no work/payment");
    }
}

#[tokio::test]
async fn upgraded_context_uses_the_actual_mutexes_atomics_and_store_for_both_gates() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let live = context.upgrade().unwrap();
    assert!(Arc::ptr_eq(&live.servers, &f.state.servers));
    assert!(Arc::ptr_eq(&live.store, &f.state.store));
    assert!(Arc::ptr_eq(&live.resumable, &f.state.session_resumable));
    assert!(Arc::ptr_eq(
        &live.requested,
        &f.state.session_lock_requested
    ));
    assert!(Arc::ptr_eq(
        &live.generation,
        &f.state.ui_session_generation
    ));
    assert!(Arc::ptr_eq(
        &live.authorized,
        &f.state.ui_authorized_generation
    ));
    assert!(Arc::ptr_eq(&live.commit, &f.state.ui_session_commit));
    assert!(Arc::ptr_eq(&live.authority, &f.state.voice_authority));
    assert!(Arc::ptr_eq(&live.ledger, &f.state.voice_handoff));
    assert!(live.gate().require_unlocked().await.is_ok());
    assert!(require_unlocked_session(&f.state).await.is_ok());
    let mounted = f.state.store.lock().await.take();
    assert!(mounted.is_some());
    assert_eq!(
        live.gate().require_unlocked().await,
        require_unlocked_session(&f.state).await
    );
    assert!(
        live.gate().require_unlocked().await.is_err(),
        "cached open flags cannot replace a real store"
    );
    *f.state.store.lock().await = mounted;
    assert!(live.gate().require_unlocked().await.is_ok());
    *f.state.session_resumable.lock().await = false;
    assert_eq!(
        live.gate().require_unlocked().await,
        require_unlocked_session(&f.state).await
    );
    assert!(live.gate().require_unlocked().await.is_err());
    *f.state.session_resumable.lock().await = true;
    let generation = live.generation.load(std::sync::atomic::Ordering::Acquire);
    live.authorized
        .store(generation + 1, std::sync::atomic::Ordering::Release);
    assert!(live.gate().require_unlocked().await.is_err());
    assert_eq!(
        live.gate().require_unlocked().await,
        require_unlocked_session(&f.state).await
    );
    live.authorized
        .store(generation, std::sync::atomic::Ordering::Release);
    assert!(live.gate().check_generation(generation).is_ok());
    assert!(live.gate().check_generation(generation + 1).is_err());
    // The context cannot carry a separate commit: holding either view blocks the other.
    let held = live.commit.lock().await;
    assert!(f.state.ui_session_commit.try_lock().is_err());
    drop(held);
    drop(live);
    f.close().await;
}

#[tokio::test]
async fn registered_lock_while_waiting_for_store_is_observed_after_the_actual_await() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let live = context.upgrade().unwrap();
    let gate = live.gate();
    let held = f.state.store.lock().await;
    let mut pending = Box::pin(gate.require_unlocked());
    assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await);
    register_ui_lock_request(&f.state);
    drop(held);
    assert_eq!(pending.as_mut().await, Err("the vault is locked".into()));
    assert!(require_unlocked_session(&f.state).await.is_err());
    drop(pending);
    crate::lock_session_inner(&f.state, None).await.unwrap();
    drop(live);
    f.close().await;
}

#[tokio::test]
async fn a_missing_late_owner_refuses_factory_before_any_actor_query_or_new_payment() {
    use crate::voice_handoff::factory::tests::{park, selection};
    use crate::voice_handoff::factory::{ensure_derivation_row, Refused};
    let f = Fixture::new().await;
    let mut context = Context::new(&f.state);
    let (_owner, selected, selector) = selection(&f).await;
    let before = strong_counts(&f.state);
    let counts = selected.resources().counts();
    context.ledger = Weak::new(); // eight earlier upgrades succeed before the ninth refuses
    let parked = park(&f.actor).await;
    let mut pending = Box::pin(ensure_derivation_row(&context, selected, selector));
    assert!(
        matches!(
            poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx))).await,
            Poll::Ready(Err(Refused::Closed))
        ),
        "dead context must not even query the parked actor"
    );
    assert_eq!(strong_counts(&f.state), before);
    assert_eq!(voice_runtime(&f.state).resources().counts(), counts);
    drop(pending);
    drop(parked);
    f.close().await;
}

#[test]
fn weak_retained_arc_layout_includes_both_tail_padding_and_overaligned_value_padding() {
    #[repr(align(64))]
    struct Aligned([u8; 1]);
    assert_eq!(
        allocation_bytes::<AtomicBool>(),
        3 * std::mem::size_of::<usize>()
    );
    assert_eq!(allocation_bytes::<Aligned>(), 128);
    // Match the pinned standard-library layout composition independently of ArcAllocation.
    let layout = std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
        .extend(std::alloc::Layout::new::<Store>())
        .unwrap()
        .0
        .pad_to_align();
    assert_eq!(allocation_bytes::<Store>(), layout.size());
}
