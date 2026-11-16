//! Native actor wiring and terminal lifecycle. There is no serialized/browser adapter or
//! audible-media claim here; successful pair/derive transitions are tested at the Root boundary.
use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::voice_v2::runtime::Runtime;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::poll_fn;

mod peer_snapshot;
mod results;
mod snapshot;

fn found(seed: u64) -> Server<MemNetwork, ChaCha20Rng> {
    Server::found(
        Hub::new().join(PeerId::from_u64(seed)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(1000)),
        "alice",
    )
    .unwrap()
}
fn point() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()
}
pub(super) async fn start() -> (
    ServerActor,
    mpsc::Receiver<TracedEvent>,
    JoinHandle<()>,
    Arc<Runtime>,
    Arc<CallGrant>,
) {
    let (actor, events, task) = spawn(found(710));
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    actor
        .prepare_voice_actor(runtime.clone(), ticket)
        .await
        .unwrap()
        .register()
        .unwrap();
    let grant = Arc::new(
        actor
            .voice_call_grant(crate::channel_id("general"))
            .await
            .unwrap(),
    );
    (actor, events, task, runtime, grant)
}
pub(super) async fn park(actor: &ServerActor) -> crate::durable_chat::DurableSendReady {
    let expected_context = actor.durable_send_context().await.unwrap();
    actor
        .prepare_durable_send(crate::durable_chat::DurableSendRequest {
            token: [31; 16],
            expected_context,
            channel: crate::channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn native_voice_actor_installs_exact_endpoint_and_coalesces_revision_observation() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let mut changes = actor.voice_endpoint_changes().await.unwrap();
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    assert!(actor
        .voice_participant_peers(grant.clone())
        .await
        .unwrap()
        .is_empty());
    let foreign = Arc::new(CallGrant::from_lease(grant.lease(), grant.scope()));
    let permit = CryptoActionSnapshotPermit::new(&runtime.resources(), grant.scope()).unwrap();
    let observed = actor
        .voice_confirmation_actions(grant.clone(), permit)
        .await
        .unwrap();
    assert!(observed.result().unwrap().is_empty());
    let refused = actor
        .voice_confirmation_actions(foreign.clone(), observed.return_permit())
        .await
        .unwrap();
    assert_eq!(refused.result().err(), Some(VoiceError::Unauthorized));
    drop(refused);
    assert_eq!(
        actor.voice_participant_peers(foreign.clone()).await.err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized)),
        "equal scope and even the same lease cannot adopt another affine participant grant"
    );
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    assert_eq!(
        actor
            .prepare_voice_pair(
                grant.clone(),
                PairSelector {
                    peer: catcoms_crypto::DeviceId::from_bytes([9; 32]),
                    join: [9; 32],
                    key_ref: [9; 32],
                    epoch: 0,
                }
            )
            .await
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized)),
        "actor commands cannot manufacture a proven pair from request metadata"
    );
    assert!(!changes.has_changed().unwrap());
    let missing = PairSelector {
        peer: catcoms_crypto::DeviceId::from_bytes([9; 32]),
        join: [9; 32],
        key_ref: [9; 32],
        epoch: 0,
    };
    assert_eq!(
        actor
            .voice_pair_confirmation(grant.clone(), missing, [0; 32])
            .await,
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    // An abandoned read-only command must not publish a wake or manufacture pair state.
    let (reply, lost) = oneshot::channel();
    drop(lost);
    actor
        .cmd_tx
        .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
            Command::PairConfirmation {
                grant: grant.clone(),
                selector: missing,
                handshake: [0; 32],
                reply,
            },
        )))
        .await
        .unwrap();
    actor.member_count().await;
    assert!(!changes.has_changed().unwrap());
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    assert!(
        !changes.has_changed().unwrap(),
        "exact retry creates no second endpoint"
    );
    let mut changed = point();
    changed[64] ^= 1;
    assert_eq!(
        actor
            .start_voice_endpoint(grant.clone(), [81; 32], changed)
            .await,
        Err(VoiceEndpointError::Protocol(VoiceError::RetryMismatch))
    );
    assert_eq!(
        actor
            .prepare_voice_derivation(grant.clone(), [0; 32])
            .await
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized)),
        "an ID cannot invent a proven pair"
    );
    actor.stop_and_wait().await.unwrap();
    assert!(!grant.lease().is_live());
    task.await.unwrap();
    drain.await.unwrap();
    assert_eq!(
        actor.voice_endpoint_changes().await.err(),
        Some(VoiceEndpointError::Stopped)
    );
    assert_eq!(
        actor.voice_pair_confirmation(grant, missing, [0; 32]).await,
        Err(VoiceEndpointError::Stopped)
    );
}

#[tokio::test]
async fn native_voice_actor_rejects_foreign_and_replaced_call_allocations() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let foreign = Arc::new(CallGrant::from_lease(CallLease::default(), grant.scope()));
    assert_eq!(
        actor.start_voice_endpoint(foreign, [81; 32], point()).await,
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    grant.lease().revoke();
    let replacement = Arc::new(
        actor
            .voice_call_grant(crate::channel_id("general"))
            .await
            .unwrap(),
    );
    actor
        .start_voice_endpoint(replacement.clone(), [81; 32], point())
        .await
        .unwrap();
    assert_eq!(
        actor.start_voice_endpoint(grant, [81; 32], point()).await,
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    assert_eq!(
        actor
            .start_voice_confirmation(replacement, [0; 32], None)
            .await
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_voice_actor_stop_survives_revocation_lost_reply_and_replacement() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let foreign = Arc::new(CallGrant::from_lease(grant.lease(), grant.scope()));
    actor.stop_voice_endpoint(foreign.clone()).await.unwrap();
    assert!(
        actor.voice_participant_peers(grant.clone()).await.is_ok(),
        "equal scope and lease cannot stop the actual affine participant"
    );

    // End wins before the actor receives cleanup. Lost reply must not suppress the admitted
    // cleanup command; the real mailbox barrier below observes its ordered completion.
    grant.lease().revoke();
    let (reply, lost) = oneshot::channel();
    drop(lost);
    actor
        .cmd_tx
        .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
            Command::Stop {
                grant: grant.clone(),
                reply,
            },
        )))
        .await
        .unwrap();
    actor.member_count().await;
    let replacement = Arc::new(
        actor
            .voice_call_grant(crate::channel_id("general"))
            .await
            .unwrap(),
    );
    actor
        .start_voice_endpoint(replacement.clone(), [82; 32], point())
        .await
        .unwrap();
    for _ in 0..3 {
        actor.stop_voice_endpoint(grant.clone()).await.unwrap();
    }
    assert!(
        actor
            .voice_participant_peers(replacement.clone())
            .await
            .is_ok(),
        "late old cleanup cannot remove the replacement participant"
    );
    actor
        .stop_voice_endpoint(replacement.clone())
        .await
        .unwrap();
    assert!(
        !replacement.lease().is_live(),
        "live exact stop revokes before dropping Root ownership"
    );
    assert_eq!(
        actor
            .voice_participant_peers(replacement.clone())
            .await
            .err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    actor.stop_voice_endpoint(replacement).await.unwrap();
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_voice_actor_lost_watch_replies_and_parent_revocation_do_not_strand_slots() {
    let (actor, mut events, task, _runtime, _grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let parent = CallLease::default();
    for _ in 0..12 {
        let (reply, lost) = oneshot::channel();
        drop(lost);
        actor
            .cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                Command::Watch {
                    room: crate::channel_id("general"),
                    parent: parent.clone(),
                    reply,
                },
            )))
            .await
            .unwrap();
        actor.member_count().await; // Actual mailbox barrier, no sleep/polling assumption.
    }
    for _ in 0..12 {
        let watch = actor
            .start_voice_watch(crate::channel_id("general"), parent.clone())
            .await
            .unwrap();
        assert!(actor
            .voice_watch_peers(watch.clone())
            .await
            .unwrap()
            .is_empty());
        drop(watch); // Last returned affine custody closes the actual observer.
    }
    let watch = actor
        .start_voice_watch(crate::channel_id("general"), parent.clone())
        .await
        .unwrap();
    parent.revoke();
    assert!(!watch.is_live());
    assert_eq!(
        actor.voice_watch_peers(watch).await.err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[test]
fn native_voice_watch_reply_lost_during_admitted_creation_closes_its_actual_grant() {
    type WatchReply = oneshot::Receiver<Result<Arc<VoiceWatchGrant>, VoiceError>>;
    #[derive(Debug)]
    struct ReplyClock {
        inner: ManualClock,
        lose: Arc<std::sync::Mutex<Option<WatchReply>>>,
    }
    impl Clock for ReplyClock {
        fn now_ms(&self) -> u64 {
            // Command::apply has already checked the real receiver. This original Root Clock
            // runs inside watch creation, before it sends the newly admitted affine grant.
            drop(self.lose.lock().unwrap().take());
            self.inner.now_ms()
        }
        fn monotonic_ms(&self) -> u64 {
            self.inner.monotonic_ms()
        }
        fn sleep(
            &self,
            duration: std::time::Duration,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            self.inner.sleep(duration)
        }
    }
    let lose = Arc::new(std::sync::Mutex::new(None));
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(713)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(713),
        Box::new(ManualClock::new(1000)),
        "alice",
    )
    .unwrap();
    let runtime = Runtime::new(Arc::new(ReplyClock {
        inner: ManualClock::new(1000),
        lose: lose.clone(),
    }));
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    server
        .prepare_voice_actor(&runtime, &ticket)
        .unwrap()
        .register()
        .unwrap();
    let mut changes = server.sync.voice_endpoint_changes().unwrap();
    let parent = CallLease::default();
    for _ in 0..12 {
        let (reply, result) = oneshot::channel();
        *lose.lock().unwrap() = Some(result);
        Command::Watch {
            room: crate::channel_id("general"),
            parent: parent.clone(),
            reply,
        }
        .apply(&mut server);
        assert!(lose.lock().unwrap().is_none());
        assert!(
            changes.has_changed().unwrap(),
            "Root published despite losing the reply during admitted creation"
        );
        changes.borrow_and_update();
    }
    // Each next native watch prunes the preceding abandoned grant. No eight-observer leak.
    assert!(server
        .sync
        .start_voice_watch(crate::channel_id("general"), parent)
        .is_ok());
}

fn saturate_events(actor: &ServerActor, events: &mut mpsc::Receiver<TracedEvent>) {
    while events.try_recv().is_ok() {}
    let sender = actor.test_events.upgrade().unwrap();
    while sender
        .try_send(TracedEvent {
            trace: Trace::NONE,
            event: AppEvent::SnapshotNeeded,
        })
        .is_ok()
    {}
    assert_eq!(sender.capacity(), 0);
}

#[tokio::test]
async fn native_voice_actor_lock_unlock_cannot_revive_original_grant_or_watch() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let watch = actor
        .start_voice_watch(crate::channel_id("general"), grant.lease())
        .await
        .unwrap();
    runtime.revoke_session();
    assert!(!grant.lease().is_live() && !watch.is_live());
    let ticket = runtime.session_ticket();
    actor
        .prepare_voice_actor(runtime.clone(), ticket.clone())
        .await
        .unwrap()
        .register()
        .unwrap();
    runtime.activate_session(&ticket).unwrap();
    let replacement = Arc::new(
        actor
            .voice_call_grant(crate::channel_id("general"))
            .await
            .unwrap(),
    );
    actor
        .start_voice_endpoint(replacement.clone(), [81; 32], point())
        .await
        .unwrap();
    assert_eq!(
        actor.start_voice_endpoint(grant, [81; 32], point()).await,
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    assert_eq!(
        actor.voice_watch_peers(watch).await.err(),
        Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    );
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_voice_stop_ack_retires_ownership_and_queued_replies_before_blocked_closed_event() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let mut other = found(711);
    let ticket = runtime.session_ticket();
    other
        .prepare_voice_actor(&runtime, &ticket)
        .unwrap()
        .register()
        .unwrap();
    let other_grant = other
        .voice_call_grant(crate::channel_id("general"))
        .unwrap();
    let changes = actor.voice_endpoint_changes().await.unwrap();
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let watch = actor
        .start_voice_watch(crate::channel_id("general"), grant.lease())
        .await
        .unwrap();
    let ready = park(&actor).await;
    while events.try_recv().is_ok() {}
    let event_sender = actor.test_events.upgrade().unwrap();
    while event_sender
        .try_send(TracedEvent {
            trace: Trace::NONE,
            event: AppEvent::SnapshotNeeded,
        })
        .is_ok()
    {}
    assert_eq!(event_sender.capacity(), 0);
    let (stopped, confirmed) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::StopAndWait { stopped })
        .await
        .unwrap();
    let (reply, queued) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
            Command::Peers {
                grant: watch.clone(),
                reply,
            },
        )))
        .await
        .unwrap();
    drop(ready);
    confirmed.await.unwrap();
    assert!(!grant.lease().is_live() && !watch.is_live());
    assert!(
        changes.has_changed().is_err(),
        "revision sender closes before Closed can be delivered"
    );
    assert!(
        queued.await.is_err(),
        "queued native grant/reply cannot be retained by the event wait"
    );
    assert!(
        !task.is_finished(),
        "full event sink still holds actual actor task"
    );
    assert!(
        other_grant.lease().is_live(),
        "another group still uses the shared Runtime"
    );
    drop(events);
    drop(event_sender);
    task.await.unwrap();
}

#[test]
fn native_voice_stop_lost_ack_after_terminal_admission_does_not_resume_original_authority() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let mut server = found(712);
    server
        .prepare_voice_actor(&runtime, &ticket)
        .unwrap()
        .register()
        .unwrap();
    let grant = server
        .voice_call_grant(crate::channel_id("general"))
        .unwrap();
    let (stopped, result) = oneshot::channel();
    assert!(admit_terminal_stop(stopped, || {
        // This runs only AFTER the helper observes a real open waiter. Lose that exact waiter
        // during admitted teardown, then exercise the same native retirement as the actor arm.
        drop(result);
        server.sync.retire_voice_actor();
    }));
    assert!(!grant.lease().is_live());
    assert!(server
        .voice_call_grant(crate::channel_id("general"))
        .is_err());
    let (stopped, cancelled) = oneshot::channel::<()>();
    drop(cancelled);
    assert!(!admit_terminal_stop(stopped, || panic!(
        "queued cancellation precedes admission"
    )));
}

#[tokio::test]
async fn native_voice_queued_stop_and_abandoned_freeze_preserve_original_installation() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let ready = park(&actor).await;
    let mut stopping = Box::pin(actor.stop_and_wait());
    poll_fn(|cx| {
        assert!(stopping.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(stopping);
    drop(ready);
    actor.member_count().await;
    assert!(grant.lease().is_live());
    let freeze = actor.prepare_shutdown().await.unwrap();
    drop(freeze);
    let freeze = actor.prepare_shutdown().await.unwrap();
    let unmounted = Arc::new(tokio::sync::Mutex::new(None));
    assert!(freeze
        .save_and_freeze(crate::shutdown::ShutdownLease::new(
            unmounted.lock_owned().await,
            1,
            ()
        ))
        .await
        .is_err());
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    assert!(
        grant.lease().is_live(),
        "cancelled freeze preserves native registration"
    );
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_voice_explicit_shutdown_retires_before_a_saturated_closed_event() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let changes = actor.voice_endpoint_changes().await.unwrap();
    let ready = park(&actor).await;
    saturate_events(&actor, &mut events);
    actor.shutdown().await;
    let (reply, queued) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
            Command::Changes { reply },
        )))
        .await
        .unwrap();
    drop(ready);
    assert!(queued.await.is_err());
    assert!(!grant.lease().is_live());
    assert!(changes.has_changed().is_err());
    assert!(
        !task.is_finished(),
        "Closed remains blocked by the actual full sink"
    );
    drop(events);
    task.await.unwrap();
}

#[tokio::test]
async fn native_voice_saved_freeze_cancel_preserves_installation_and_stop_retires_before_closed_event(
) {
    let (actor, mut events, task, _runtime, grant) = start().await;
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let changes = actor.voice_endpoint_changes().await.unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let vault = Arc::new(tokio::sync::Mutex::new(Some(
        crate::ServerStore::open(
            temporary.path(),
            b"test",
            &mut ChaCha20Rng::seed_from_u64(714),
        )
        .unwrap(),
    )));
    let ready = actor.prepare_shutdown().await.unwrap();
    let frozen = ready
        .save_and_freeze(crate::shutdown::ShutdownLease::new(
            vault.clone().lock_owned().await,
            1,
            (),
        ))
        .await
        .unwrap();
    drop(frozen);
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    assert!(
        grant.lease().is_live(),
        "cancel after actual save preserves native registration"
    );
    let ready = actor.prepare_shutdown().await.unwrap();
    let frozen = ready
        .save_and_freeze(crate::shutdown::ShutdownLease::new(
            vault.lock_owned().await,
            1,
            (),
        ))
        .await
        .unwrap();
    saturate_events(&actor, &mut events);
    let (reply, queued) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
            Command::Changes { reply },
        )))
        .await
        .unwrap();
    frozen.stop();
    assert!(queued.await.is_err());
    assert!(!grant.lease().is_live());
    assert!(changes.has_changed().is_err());
    assert!(
        !task.is_finished(),
        "Closed remains blocked after successful saved stop"
    );
    drop(events);
    task.await.unwrap();
}
