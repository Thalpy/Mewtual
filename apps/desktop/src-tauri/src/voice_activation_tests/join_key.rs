//! Real native actor/bridge boundaries; this is not WebView/RTC or physical audio acceptance.
use super::*;
// Pressure must track the actual native policy, including future reviewed policy revisions.
// A historical256KiB filler no longer exercises capacity refusal at the current512KiB limit.
use crate::voice_join_key::{admit_inner, publish_inner, settle_inner};
use catcoms_sync::voice_v2::resources::GROUP_BYTES;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

fn executor() -> String {
    hex::encode([9; 32])
}
fn point() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()
}
async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}
async fn owner(f: &Fixture) -> String {
    let id = f.candidate().await;
    activate_voice_owner_inner(&f.state, id.clone())
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn selected_join_key_actual_endpoint_recovers_publication_without_replacing_join() {
    let f = Fixture::new().await;
    let owner = owner(&f).await;
    let first = admit_inner(&f.state, &owner, &executor()).await.unwrap();
    let retry = admit_inner(&f.state, &owner, &executor()).await.unwrap();
    assert_eq!(
        (first.permit.clone(), first.join.clone()),
        (retry.permit, retry.join)
    );
    let published = publish_inner(&f.state, &owner, &executor(), &first.permit, point())
        .await
        .unwrap();
    settle_inner(&f.state, &owner, [9; 32]).await.unwrap();
    // Settlement is a recomputation fence, not a reason to forget already accepted public history.
    let recovered = publish_inner(&f.state, &owner, &executor(), &first.permit, point())
        .await
        .unwrap();
    assert_eq!(
        (published.join, published.key_ref),
        (recovered.join, recovered.key_ref)
    );
    assert_eq!(
        admit_inner(&f.state, &owner, &executor())
            .await
            .unwrap()
            .kind,
        "closed"
    );
    let mut changed = point();
    changed[1] ^= 1;
    assert!(
        publish_inner(&f.state, &owner, &executor(), &first.permit, changed)
            .await
            .is_err()
    );
    let refused = admit_inner(&f.state, &owner, &hex::encode([8; 32]))
        .await
        .unwrap();
    assert_eq!(refused.kind, "refused");
    assert!(refused.permit.is_empty() && refused.join.is_empty());
    f.close().await;
}

#[tokio::test]
async fn selected_join_key_real_root_capacity_retries_the_original_point() {
    let f = Fixture::new().await;
    let owner = owner(&f).await;
    let admitted = admit_inner(&f.state, &owner, &executor()).await.unwrap();
    let (resources, group) = {
        let authority = f.state.voice_authority.lock().unwrap();
        (
            authority.runtime().resources(),
            authority.owner(&owner).unwrap().binding.scope.group,
        )
    };
    let pressure = resources
        .charge(
            group,
            GROUP_BYTES - resources.counts().0,
            catcoms_sync::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    assert!(
        publish_inner(&f.state, &owner, &executor(), &admitted.permit, point())
            .await
            .is_err()
    );
    let mut changed = point();
    changed[1] ^= 1;
    assert!(
        publish_inner(&f.state, &owner, &executor(), &admitted.permit, changed)
            .await
            .is_err()
    );
    drop(pressure);
    publish_inner(&f.state, &owner, &executor(), &admitted.permit, point())
        .await
        .unwrap();
    settle_inner(&f.state, &owner, [9; 32]).await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn selected_join_key_lock_wins_before_admission_and_unlock_cannot_revive_it() {
    let f = Fixture::new().await;
    let owner = owner(&f).await;
    let boundary = f.state.ui_session_commit.lock().await;
    let executor_id = executor();
    let (admitted, ()) = tokio::join!(biased;
        admit_inner(&f.state, &owner, &executor_id),
        async {
            f.actor.member_count().await;
            register_ui_lock_request(&f.state);
            drop(boundary);
        }
    );
    assert!(admitted.is_err());
    lock_session_inner(&f.state, None).await.unwrap();
    let generation = f.state.ui_session_generation.load(Ordering::Acquire);
    finalize_unlock_session(&f.state, generation, false)
        .await
        .unwrap();
    assert!(admit_inner(&f.state, &owner, &executor()).await.is_err());
    settle_inner(&f.state, &owner, [9; 32]).await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn selected_join_key_admitted_publication_survives_canceled_waiter_and_holds_lock() {
    let f = Fixture::new().await;
    let owner = owner(&f).await;
    let admitted = admit_inner(&f.state, &owner, &executor()).await.unwrap();
    let executor_id = executor();
    let boundary = f.state.ui_session_commit.lock().await;
    let mut publication = Box::pin(publish_inner(
        &f.state,
        &owner,
        &executor_id,
        &admitted.permit,
        point(),
    ));
    assert!(poll_once(publication.as_mut()).await.is_pending());
    f.actor.member_count().await;
    assert!(poll_once(publication.as_mut()).await.is_pending());
    // This production durable-send handshake parks the real actor after Ready, without writing
    // or replacing a voice handler. Dropping Ready cancels it and releases the queued voice turn.
    let context = f.actor.durable_send_context().await.unwrap();
    let parked = f
        .actor
        .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
            token: [31; 16],
            expected_context: context,
            channel: channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap();
    drop(boundary);
    assert!(poll_once(publication.as_mut()).await.is_pending());
    drop(publication);
    assert!(
        f.state.ui_session_commit.try_lock().is_err(),
        "the native worker owns commit after invoke cancellation"
    );
    let mut settlement = Box::pin(settle_inner(&f.state, &owner, [9; 32]));
    assert!(
        poll_once(settlement.as_mut()).await.is_pending(),
        "settlement awaits actual publication drainage"
    );
    let mut lock = Box::pin(lock_session_inner(&f.state, None));
    assert!(
        poll_once(lock.as_mut()).await.is_pending(),
        "lock cannot finish ahead of the admitted native worker"
    );
    drop(parked);
    settlement.as_mut().await.unwrap();
    lock.as_mut().await.unwrap();
    drop(settlement);
    drop(lock);
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .owner(&owner)
        .is_none());
    f.close().await;
}
