//! Actual native bridge/actor rejection and deterministic lifecycle barriers. Successful
//! proven-pair Actor registration remains unproved while normal V2 ingress is disabled.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::{
    activate_voice_owner_inner, finalize_unlock_session, lock_session_inner,
    register_ui_lock_request,
};
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}
pub(in crate::voice_handoff) async fn selection(
    f: &Fixture,
) -> (String, Arc<SelectedHandoffOwner>, PairSelector) {
    let owner = f.candidate().await;
    activate_voice_owner_inner(&f.state, owner.clone())
        .await
        .unwrap();
    let executor = hex::encode([9; 32]);
    let admission = crate::voice_join_key::admit_inner(&f.state, &owner, &executor)
        .await
        .unwrap();
    let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
    crate::voice_join_key::publish_inner(&f.state, &owner, &executor, &admission.permit, point)
        .await
        .unwrap();
    crate::voice_join_key::settle_inner(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .selected_handoff_owner(&owner, [9; 32])
        .unwrap();
    let selector = PairSelector {
        peer: catcoms_crypto::DeviceId::from_bytes([91; 32]),
        join: [8; 32],
        key_ref: [7; 32],
        epoch: 0,
    };
    (owner, selected, selector)
}
pub(in crate::voice_handoff) async fn park(
    actor: &ServerActor,
) -> catcoms_app::durable_chat::DurableSendReady {
    let expected_context = actor.durable_send_context().await.unwrap();
    actor
        .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
            token: [71; 16],
            expected_context,
            channel: catcoms_app::channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn registration_rejection_retries_the_same_counted_native_purpose() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_owner, selected, selector) = selection(&f).await;
    assert!(matches!(
        ensure_derivation_row(&context, selected.clone(), selector).await,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    let Begin::Factory(first) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("retry factory");
    };
    let first_id = first.id();
    drop(first);
    assert!(matches!(
        ensure_derivation_row(&context, selected.clone(), selector).await,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    let Begin::Factory(retry) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("same retry factory");
    };
    assert!(retry.id() == first_id);
    drop(retry);
    assert!(f
        .state
        .voice_handoff
        .resolve(&selected, Purpose::Derive(selector), first_id)
        .unwrap()
        .status()
        .unwrap()
        .action
        .is_none());
    f.close().await;
}

#[tokio::test]
async fn registration_final_generation_fence_refuses_a_registered_lock_before_its_hook_runs() {
    let f = Fixture::new().await;
    let (_owner, selected, _) = selection(&f).await;
    {
        let authority = f.state.voice_authority.lock().unwrap();
        require_current_selection(f.state.ui_session_gate(), &authority, &selected).unwrap();
        register_ui_lock_request(&f.state);
        assert!(
            selected.lease().is_live(),
            "revocation hook is deliberately held behind this same Coordinator guard"
        );
        assert_eq!(
            require_current_selection(f.state.ui_session_gate(), &authority, &selected),
            Err(Refused::Closed)
        );
    }
    lock_session_inner(&f.state, None).await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn registration_pending_duplicate_returns_only_original_metadata_without_another_actor_turn()
{
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_owner, selected, selector) = selection(&f).await;
    let boundary = f.state.ui_session_commit.lock().await;
    let mut pending = Box::pin(ensure_derivation_row(&context, selected.clone(), selector));
    assert!(poll_once(pending.as_mut()).await.is_pending());
    f.actor.member_count().await;
    assert!(poll_once(pending.as_mut()).await.is_pending());
    let parked = park(&f.actor).await;
    let Begin::Factory(original) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("original");
    };
    original.mark_handoff().unwrap();
    let id = original.id();
    drop(boundary);
    assert!(
        matches!(poll_once(pending.as_mut()).await, Poll::Ready(Ok(recovered)) if recovered == id)
    );
    assert!(
        f.state
            .voice_handoff
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap()
            .status()
            .unwrap()
            .action
            .is_none(),
        "Pending RowId does not imply attached input/readiness"
    );
    drop(pending);
    drop(original);
    drop(parked);
    f.close().await;
}

#[tokio::test]
async fn registration_payment_refusal_before_handoff_calls_no_actor_and_removes_unused_row() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_owner, selected, selector) = selection(&f).await;
    let boundary = f.state.ui_session_commit.lock().await;
    let mut pending = Box::pin(ensure_derivation_row(&context, selected.clone(), selector));
    assert!(poll_once(pending.as_mut()).await.is_pending());
    f.actor.member_count().await;
    assert!(poll_once(pending.as_mut()).await.is_pending());
    let parked = park(&f.actor).await;
    let pool = selected.resources();
    // Leave room for the fixed row+Phase, but less than the independent factory payment.
    let pressure = pool
        .charge(
            selected.binding().scope.group,
            256 * 1024 - pool.counts().0 - 4096 - 1024,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    drop(boundary);
    assert!(
        matches!(
            poll_once(pending.as_mut()).await,
            Poll::Ready(Err(Refused::Protocol(VoiceError::Capacity)))
        ),
        "must finish without waiting for the parked actor"
    );
    assert_eq!(
        pool.counts(),
        before,
        "unused fixed row and Phase returned; no actor work was started"
    );
    drop(pending);
    drop(pressure);
    drop(parked);
    f.close().await;
}

#[tokio::test]
async fn registration_detached_worker_holds_commit_after_caller_loss_and_unlock_cannot_revive_it() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_owner, selected, selector) = selection(&f).await;
    let broad_counts = || {
        [
            Arc::strong_count(&f.state.servers),
            Arc::strong_count(&f.state.store),
            Arc::strong_count(&f.state.voice_authority),
            Arc::strong_count(&f.state.session_resumable),
            Arc::strong_count(&f.state.session_lock_requested),
            Arc::strong_count(&f.state.ui_session_generation),
            Arc::strong_count(&f.state.ui_authorized_generation),
        ]
    };
    let before = broad_counts();
    let boundary = f.state.ui_session_commit.lock().await;
    let mut pending = Box::pin(ensure_derivation_row(&context, selected.clone(), selector));
    assert!(poll_once(pending.as_mut()).await.is_pending());
    f.actor.member_count().await;
    assert!(poll_once(pending.as_mut()).await.is_pending());
    let parked = park(&f.actor).await;
    drop(boundary);
    assert!(poll_once(pending.as_mut()).await.is_pending());
    drop(pending); // dropping JoinHandle's waiter does not abort actual admitted native work
    assert_eq!(
        broad_counts(),
        before,
        "worker cannot retain broad native context owners"
    );
    assert!(f.state.ui_session_commit.try_lock().is_err());
    let mut lock = Box::pin(lock_session_inner(&f.state, None));
    assert!(poll_once(lock.as_mut()).await.is_pending());
    drop(parked);
    lock.as_mut().await.unwrap();
    drop(lock);
    let generation = f
        .state
        .ui_session_generation
        .load(std::sync::atomic::Ordering::Acquire);
    finalize_unlock_session(&f.state, generation, false)
        .await
        .unwrap();
    assert!(matches!(
        ensure_derivation_row(&context, selected.clone(), selector).await,
        Err(Refused::Closed)
    ));
    f.close().await;
}

#[tokio::test]
async fn registration_completed_uncollected_error_keeps_factory_payment_until_collection() {
    let f = Fixture::new().await;
    let (_owner, selected, selector) = selection(&f).await;
    let Begin::Factory(registration) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("factory");
    };
    let pool = selected.resources();
    let before = pool.counts();
    let credit = pool
        .charge(
            selected.binding().scope.group,
            FACTORY_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    let commit = f.state.ui_session_commit.clone().lock_owned().await;
    let grant = selected.grant().unwrap();
    let work = run(
        registration,
        selected,
        f.actor.clone(),
        grant,
        selector,
        commit,
        credit,
    );
    let bytes = worker_layout(&work);
    assert!(
        bytes <= FACTORY_BYTES,
        "actual factory layout {bytes} exceeds {FACTORY_BYTES}"
    );
    // A panicking worker drops Sender and wakes this assertion with an error; Notify alone
    // would leave the regression waiting forever instead of exposing that worker failure.
    let (signal, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let output = work.await;
        let _ = signal.send(());
        output
    });
    ready
        .await
        .expect("factory worker panicked before producing output");
    assert_eq!(pool.counts(), (before.0 + FACTORY_BYTES, before.1));
    let output = task.await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert_eq!(pool.counts(), (before.0 + FACTORY_BYTES, before.1));
    drop(output);
    assert_eq!(pool.counts(), before);
    f.close().await;
}

#[tokio::test]
async fn actual_store_removed_during_commit_wait_refuses_before_ledger_or_actor_admission() {
    let f = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_owner, selected, selector) = selection(&f).await;
    let boundary = f.state.ui_session_commit.lock().await;
    let before = selected.resources().counts();
    let mut pending = Box::pin(ensure_derivation_row(&context, selected, selector));
    assert!(poll_once(pending.as_mut()).await.is_pending());
    f.actor.member_count().await; // the preceding exact context query has completed
    assert!(poll_once(pending.as_mut()).await.is_pending());
    let parked = park(&f.actor).await;
    let mounted = f.state.store.lock().await.take();
    assert!(mounted.is_some());
    drop(boundary);
    assert!(
        matches!(
            poll_once(pending.as_mut()).await,
            Poll::Ready(Err(Refused::Closed))
        ),
        "store absence must refuse without waiting for the parked actor"
    );
    assert_eq!(crate::voice_runtime(&f.state).resources().counts(), before);
    drop(pending);
    *f.state.store.lock().await = mounted;
    drop(parked);
    f.close().await;
}

#[tokio::test]
async fn original_selected_allocation_cannot_be_substituted_by_equal_public_owner_identity() {
    let f = Fixture::new().await;
    let foreign = Fixture::new().await;
    let context = Context::new(&f.state);
    let (_owner, selected, _) = selection(&f).await;
    let (_foreign_owner, other, selector) = selection(&foreign).await;
    assert_eq!(selected.id(), other.id());
    assert_eq!(selected.executor(), other.executor());
    let before = selected.resources().counts();
    let parked = park(&f.actor).await;
    let mut pending = Box::pin(ensure_derivation_row(&context, other, selector));
    assert!(matches!(
        poll_once(pending.as_mut()).await,
        Poll::Ready(Err(Refused::Closed))
    ));
    assert_eq!(selected.resources().counts(), before);
    drop(pending);
    drop(parked);
    f.close().await;
    foreign.close().await;
}
