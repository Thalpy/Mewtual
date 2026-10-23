//! The original affine snapshot payment follows the actual command and late reply, rather
//! than the waiting caller. These tests pin cancellation on both sides of Actor admission.
use super::*;
use std::sync::Mutex;

#[tokio::test]
async fn stopped_actor_keeps_snapshot_payment_until_the_parked_receiver_is_collected() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    let (stopped, confirmed) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::StopAndWait { stopped })
        .await
        .unwrap();
    let resources = runtime.resources();
    let permit = CryptoActionSnapshotPermit::new(&resources, grant.scope()).unwrap();
    let mut request = Box::pin(actor.voice_confirmation_actions(grant.clone(), permit));
    poll_fn(|cx| {
        assert!(request.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(parked);
    confirmed.await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
    // Stop destroyed the queued command and its sender. Do NOT poll/drop the receiver yet:
    // its oneshot allocation is still real even though no output will ever be sent.
    let held = resources.counts();
    assert!(held.0 >= CryptoActionSnapshotPermit::MEMORY_BYTES);
    drop(request);
    assert_eq!(
        resources.counts().0,
        held.0 - CryptoActionSnapshotPermit::MEMORY_BYTES
    );
}

#[tokio::test]
async fn queued_snapshot_caller_loss_keeps_payment_until_command_collection() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    // This command is cancelled BEFORE Root admission, so it never checks pool authority.
    // Isolate its payment from the live Actor's unrelated startup/maintenance collection.
    let resources = Arc::new(catcoms_sync::voice_v2::resources::Resources::default());
    let before = resources.counts();
    let permit = CryptoActionSnapshotPermit::new(&resources, grant.scope()).unwrap();
    let charged = resources.counts();
    assert_eq!(
        charged.0,
        before.0 + CryptoActionSnapshotPermit::MEMORY_BYTES
    );
    let mut request = Box::pin(actor.voice_confirmation_actions(grant, permit));
    poll_fn(|cx| {
        assert!(request.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(request);
    assert_eq!(
        resources.counts(),
        charged,
        "caller loss cannot release an actual queued command"
    );
    drop(parked);
    actor.member_count().await;
    assert_eq!(resources.counts(), before);
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn snapshot_success_and_refusal_retain_payment_through_uncollected_and_held_output() {
    let (actor, mut events, task, runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let resources = runtime.resources();
    for refused in [false, true] {
        let owner = if refused {
            Arc::new(CallGrant::from_lease(grant.lease(), grant.scope()))
        } else {
            grant.clone()
        };
        let permit = CryptoActionSnapshotPermit::new(&resources, owner.scope()).unwrap();
        let (reply, completed) = oneshot::channel();
        actor
            .cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                Command::ConfirmationActions {
                    grant: owner,
                    permit,
                    reply,
                },
            )))
            .await
            .unwrap();
        actor.member_count().await; // Real mailbox barrier: the reply is ready, not collected.
                                    // Park the actual Actor before measuring last-drop deltas. Startup/directory/source
                                    // collection legitimately changes OTHER payments during this successful command.
        let parked = park(&actor).await;
        let charged = resources.counts();
        let output = completed.await.unwrap().unwrap();
        if refused {
            assert_eq!(output.result().err(), Some(VoiceError::Unauthorized));
        } else {
            assert!(output.result().unwrap().is_empty());
        }
        assert_eq!(
            resources.counts(),
            charged,
            "even a refusal owns its completed reply metadata"
        );
        let permit = output.return_permit();
        assert_eq!(
            resources.counts(),
            charged,
            "only the original payment is reused after Vec collection"
        );
        drop(permit);
        assert_eq!(
            resources.counts().0,
            charged.0 - CryptoActionSnapshotPermit::MEMORY_BYTES
        );
        drop(parked);
    }
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[test]
fn admitted_snapshot_caller_loss_cannot_release_payment_before_root_finishes() {
    type Gate = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);
    #[derive(Debug)]
    struct AdmissionClock {
        inner: ManualClock,
        gate: Mutex<Option<Gate>>,
    }
    impl Clock for AdmissionClock {
        fn now_ms(&self) -> u64 {
            let gate = self.gate.lock().unwrap().take();
            if let Some((entered, release)) = gate {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
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
    let clock = Arc::new(AdmissionClock {
        inner: ManualClock::new(1000),
        gate: Mutex::new(None),
    });
    let runtime = Runtime::new(clock.clone());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let mut server = found(716);
    server
        .prepare_voice_actor(&runtime, &ticket)
        .unwrap()
        .register()
        .unwrap();
    let grant = Arc::new(
        server
            .voice_call_grant(crate::channel_id("general"))
            .unwrap(),
    );
    server
        .sync
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .unwrap();
    let resources = runtime.resources();
    let before = resources.counts();
    let permit = CryptoActionSnapshotPermit::new(&resources, grant.scope()).unwrap();
    let charged = resources.counts();
    let (reply, completed) = oneshot::channel();
    let (entered, admission) = std::sync::mpsc::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    *clock.gate.lock().unwrap() = Some((entered, proceed));
    std::thread::scope(|scope| {
        let work = scope.spawn(|| {
            Command::ConfirmationActions {
                // Retain the original affine call in this test. Dropping its last owner is
                // separately paid call retirement, not a snapshot-payment observation.
                grant: grant.clone(),
                permit,
                reply,
            }
            .apply(&mut server)
        });
        admission.recv().unwrap(); // Root Clock runs AFTER Command's receiver-is-open check.
        drop(completed);
        assert_eq!(
            resources.counts(),
            charged,
            "an admitted request still owns the original payment"
        );
        release.send(()).unwrap();
        work.join().unwrap();
    });
    assert_eq!(
        resources.counts(),
        before,
        "the failed late send physically collects its paid output"
    );
}

#[tokio::test]
async fn snapshot_complete_wrapper_inventory_fits_original_payment() {
    let (actor, events, task, runtime, grant) = start().await;
    let permit = CryptoActionSnapshotPermit::new(&runtime.resources(), grant.scope()).unwrap();
    let future = actor.voice_confirmation_actions(grant, permit);
    // Count the caller future AND actual mailbox envelope AND reply allocation/output,
    // including a full internal32-handle backing Vec although at most four are returned.
    let bytes = std::mem::size_of_val(&future)
        + std::mem::size_of::<crate::AppCommand>()
        + std::mem::size_of::<CryptoActionSnapshotPermit>()
        + std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::CryptoActionSnapshotWaiter>()
        + CryptoActionSnapshotPermit::SHARED_OWNER_BYTES
        + std::mem::size_of::<CryptoActionSnapshot>()
        + std::mem::size_of::<Result<CryptoActionSnapshot, VoiceError>>()
        + std::mem::size_of::<oneshot::Receiver<Result<CryptoActionSnapshot, VoiceError>>>()
        + 32 * std::mem::size_of::<Arc<CryptoAction>>()
        + 1024; // Pinned Tokio oneshot/envelope bookkeeping; no detached task.
    assert!(
        bytes <= CryptoActionSnapshotPermit::MEMORY_BYTES,
        "snapshot metadata requires {bytes} bytes"
    );
    drop(future);
    actor.stop_and_wait().await.unwrap();
    drop(events);
    task.await.unwrap();
}
