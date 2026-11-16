//! Actual actor/mailbox ownership with a real but FOREIGN LocalJoin action. Rejection is
//! intentional: these tests do not manufacture a proven pair inside the receiving Root.
//! Successful protected seal/open transitions remain covered at that Root boundary.
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_sync::voice_v2::limits::OriginLimits;
use catcoms_sync::voice_v2::packets::Authority;
use catcoms_sync::voice_v2::presence::{LocalJoin, WatchAuthority, Watcher};
use catcoms_sync::voice_v2::resources::{ByteBudget, Resources};
use catcoms_sync::voice_v2::{Context, JoinKey, WatchLease};
use std::sync::atomic::{AtomicUsize, Ordering};

fn foreign_action(grant: &CallGrant) -> (LocalJoin, Arc<CryptoAction>) {
    let clock = Arc::new(ManualClock::new(1000));
    let pool = Arc::new(Resources::default());
    let mut rng = ChaCha20Rng::seed_from_u64(721);
    let signer = DeviceKeypair::from_seed(&[3; 32]);
    let peer = DeviceKeypair::from_seed(&[4; 32]);
    let context = Context {
        scope: grant.scope(),
        local: signer.device_id(),
        now: 1000,
        epoch: 1,
        indirect: true,
    };
    let remote = Context {
        local: peer.device_id(),
        ..context
    };
    let remote_lease = CallLease::default();
    let members = |_| true;
    // Its OWN actual call lease makes this a genuinely foreign LocalJoin. Reusing the
    // actor's lease would correctly refuse a second join binding before testing the bridge.
    let lease = CallLease::default();
    let authority = Authority {
        context: &context,
        lease: &lease,
        member: &members,
    };
    let peer_authority = Authority {
        context: &remote,
        lease: &remote_lease,
        member: &members,
    };
    let mut join = LocalJoin::new(
        &context,
        JoinKey::new([11; 32], point()).unwrap(),
        lease.clone(),
        pool.clone(),
    )
    .unwrap();
    let mut other = LocalJoin::new(
        &remote,
        JoinKey::new([12; 32], point()).unwrap(),
        remote_lease.clone(),
        pool.clone(),
    )
    .unwrap();
    join.announce(&signer, &authority, clock.as_ref(), &mut rng)
        .unwrap();
    let announcement = other
        .announce(&peer, &peer_authority, clock.as_ref(), &mut rng)
        .unwrap();
    let watch = WatchLease::default();
    let wa = WatchAuthority {
        context: &context,
        lease: &watch,
        member: &members,
    };
    let mut watcher = Watcher::new(&context, watch.clone(), pool.clone(), &mut rng).unwrap();
    let mut limits = OriginLimits::new(context.scope.group, pool);
    let challenge = watcher
        .challenge(
            &announcement,
            &signer,
            &wa,
            &mut limits,
            clock.as_ref(),
            &mut rng,
        )
        .unwrap();
    let proof = other
        .answer(&challenge, &peer, &peer_authority, clock.as_ref(), &mut rng)
        .unwrap();
    let fresh = watcher.prove(&proof, &wa, clock.as_ref()).unwrap();
    let pair = join
        .begin_pair(&fresh, &authority, &mut limits, clock.as_ref())
        .unwrap();
    let action = join
        .derivation_action(pair.handshake, &authority, clock, ByteBudget::Ordinary)
        .unwrap();
    (join, action)
}

struct OwnedResult {
    bytes: Vec<u8>,
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl AsRef<[u8]> for OwnedResult {
    fn as_ref(&self) -> &[u8] {
        self.calls.fetch_add(1, Ordering::SeqCst);
        &self.bytes
    }
}
impl Drop for OwnedResult {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn body() -> (VoiceConfirmationBody, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let body = VoiceConfirmationBody::from_owner(OwnedResult {
        bytes: vec![4; 32],
        calls: calls.clone(),
        drops: drops.clone(),
    })
    .unwrap();
    (body, calls, drops)
}

#[tokio::test]
async fn completed_uncollected_actor_reply_retains_original_body_and_never_reinvokes_its_callback()
{
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let (_foreign, action) = foreign_action(&grant);
    for open in [false, true] {
        let (body, calls, drops) = body();
        let (reply, completed) = oneshot::channel();
        actor
            .cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                Command::ConfirmationResult {
                    grant: grant.clone(),
                    action: action.clone(),
                    body: body.claim().unwrap(),
                    tag: [8; 32],
                    open,
                    reply,
                },
            )))
            .await
            .unwrap();
        drop(body);
        actor.member_count().await; // Actual actor turn completed; result is still uncollected.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let completion = completed.await.unwrap().unwrap();
        assert!(
            completion.result().is_err(),
            "a foreign action cannot acquire Root authority"
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(completion);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn caller_loss_while_queued_keeps_the_body_until_the_actual_actor_collects_the_command() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_foreign, action) = foreign_action(&grant);
    let ready = park(&actor).await;
    let (body, calls, drops) = body();
    let mut request =
        Box::pin(actor.submit_voice_confirmation_result(grant, action, &body, [8; 32]));
    poll_fn(|cx| {
        assert!(request.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(request);
    drop(body);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "the queued command still owns the original buffer"
    );
    drop(ready);
    actor.member_count().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn repeated_rejection_and_stopped_actor_keep_the_callers_exact_body_available_for_retry() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_foreign, action) = foreign_action(&grant);
    let (body, calls, drops) = body();
    for _ in 0..3 {
        let completion = actor
            .commit_voice_confirmation_result(grant.clone(), action.clone(), &body, [8; 32])
            .await
            .unwrap();
        assert!(completion.result().is_err());
        drop(completion);
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "rejection does not consume the original body"
        );
    }
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
    assert_eq!(
        actor
            .submit_voice_confirmation_result(grant, action, &body, [8; 32])
            .await
            .err(),
        Some(VoiceEndpointError::Stopped)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(body);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn uncollected_completion_refuses_another_request_until_its_actual_flight_is_released() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_foreign, action) = foreign_action(&grant);
    let (body, calls, drops) = body();
    let completed = actor
        .submit_voice_confirmation_result(grant.clone(), action.clone(), &body, [8; 32])
        .await
        .unwrap();
    let ready = park(&actor).await;
    assert_eq!(
        actor
            .commit_voice_confirmation_result(grant.clone(), action.clone(), &body, [8; 32])
            .await
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Capacity)),
        "local capacity refusal does not even wait for the parked actor"
    );
    drop(completed);
    drop(ready);
    let completed = actor
        .submit_voice_confirmation_result(grant, action, &body, [8; 32])
        .await
        .unwrap();
    drop(body);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(completed);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn actual_seal_and_open_request_futures_fit_the_original_body_metadata_contribution() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_foreign, action) = foreign_action(&grant);
    let (body, _calls, _drops) = body();
    let seal =
        actor.submit_voice_confirmation_result(grant.clone(), action.clone(), &body, [8; 32]);
    let open =
        actor.commit_voice_confirmation_result(grant.clone(), action.clone(), &body, [8; 32]);
    let bytes = crate::actor::voice_endpoint::body::metadata_layout(
        std::mem::size_of_val(&seal).max(std::mem::size_of_val(&open)),
    );
    assert!(
        bytes <= VOICE_CONFIRMATION_BODY_METADATA_BYTES,
        "actual actor body/future metadata requires {bytes}"
    );
    drop((seal, open)); // Never polled: no actor request or engine admission.
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}
