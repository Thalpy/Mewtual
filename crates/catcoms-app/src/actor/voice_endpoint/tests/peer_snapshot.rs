//! Both halves of the peer RPC retain the same affine payment. A stopped sender and a
//! missing receiver are independent collection boundaries, not successful observation.
use super::*;
use catcoms_sync::voice_v2::presence::PeerSnapshotWaiter;

#[tokio::test]
async fn queued_peer_snapshot_caller_loss_retains_the_actual_command_payment() {
    let (actor, mut events, task, _runtime, grant) = start().await;
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    actor
        .start_voice_endpoint(grant.clone(), [81; 32], point())
        .await
        .unwrap();
    let parked = park(&actor).await;
    let resources = Arc::new(catcoms_sync::voice_v2::resources::Resources::default());
    let before = resources.counts();
    let permit = PeerSnapshotPermit::new(&resources, grant.scope()).unwrap();
    let charged = resources.counts();
    let mut pending = Box::pin(actor.voice_participant_peer_snapshot(grant, permit));
    poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(pending);
    assert_eq!(resources.counts(), charged);
    drop(parked);
    actor.member_count().await;
    assert_eq!(resources.counts(), before);
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn stopped_actor_peer_snapshot_keeps_unpolled_receiver_paid() {
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
    let permit = PeerSnapshotPermit::new(&resources, grant.scope()).unwrap();
    let mut pending = Box::pin(actor.voice_participant_peer_snapshot(grant, permit));
    poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(parked);
    confirmed.await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
    let held = resources.counts();
    assert!(held.0 >= PeerSnapshotPermit::MEMORY_BYTES);
    drop(pending);
    assert_eq!(
        resources.counts().0,
        held.0 - PeerSnapshotPermit::MEMORY_BYTES
    );
}

#[tokio::test]
async fn peer_snapshot_success_and_refusal_retain_uncollected_output_and_exact_permit() {
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
        let permit = PeerSnapshotPermit::new(&resources, owner.scope()).unwrap();
        let waiter = permit.rpc_waiter().unwrap();
        assert_eq!(permit.rpc_waiter().err(), Some(VoiceError::Capacity));
        let (reply, completed) = oneshot::channel();
        actor
            .cmd_tx
            .send(AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                Command::ParticipantPeerSnapshot {
                    grant: owner,
                    permit,
                    reply,
                },
            )))
            .await
            .unwrap();
        actor.member_count().await;
        let parked = park(&actor).await;
        let charged = resources.counts();
        let output = completed.await.unwrap().unwrap();
        if refused {
            assert_eq!(output.result().err(), Some(VoiceError::Unauthorized));
        } else {
            assert!(output.result().unwrap().1.is_empty());
        }
        assert_eq!(resources.counts(), charged);
        let permit = output.return_permit();
        assert_eq!(permit.rpc_waiter().err(), Some(VoiceError::Capacity));
        drop(permit);
        assert_eq!(
            resources.counts(),
            charged,
            "the original receiver still owns its metadata"
        );
        drop(waiter);
        assert_eq!(
            resources.counts().0,
            charged.0 - PeerSnapshotPermit::MEMORY_BYTES
        );
        drop(parked);
    }
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn complete_peer_rpc_wrapper_inventory_fits_its_unchanged_payment() {
    let (actor, events, task, runtime, grant) = start().await;
    let permit = PeerSnapshotPermit::new(&runtime.resources(), grant.scope()).unwrap();
    let future = actor.voice_participant_peer_snapshot(grant, permit);
    let bytes = std::mem::size_of_val(&future)
        + std::mem::size_of::<crate::AppCommand>()
        + std::mem::size_of::<PeerSnapshotPermit>()
        + std::mem::size_of::<PeerSnapshotWaiter>()
        + PeerSnapshotPermit::SHARED_OWNER_BYTES
        + std::mem::size_of::<PeerSnapshot>()
        + std::mem::size_of::<Result<PeerSnapshot, VoiceError>>()
        + std::mem::size_of::<oneshot::Receiver<Result<PeerSnapshot, VoiceError>>>()
        + PeerSnapshotPermit::MAX_PEERS * std::mem::size_of::<FreshPeer>()
        + 1024;
    assert!(
        bytes <= PeerSnapshotPermit::MEMORY_BYTES,
        "peer snapshot requires {bytes}"
    );
    drop(future);
    actor.stop_and_wait().await.unwrap();
    drop(events);
    task.await.unwrap();
}
