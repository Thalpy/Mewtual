use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::poll_fn;

fn start() -> (ServerActor, mpsc::Receiver<TracedEvent>, JoinHandle<()>) {
    let server = Server::found(
        Hub::new().join(PeerId::from_u64(901)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(901),
        Box::new(ManualClock::new(1_000)),
        "alice",
    )
    .unwrap();
    spawn(server)
}

async fn park(actor: &ServerActor) -> crate::durable_chat::DurableSendReady {
    let context = actor.durable_send_context().await.unwrap();
    actor
        .prepare_durable_send(crate::durable_chat::DurableSendRequest {
            token: [1; 16],
            expected_context: context,
            channel: crate::channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn acknowledged_stop_rejects_queued_and_subsequent_authoring() {
    let (actor, mut events, task) = start();
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let ready = park(&actor).await;
    let (stopped, confirmed) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::StopAndWait { stopped })
        .await
        .unwrap();
    let (reply, late_result) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::SendMessage {
            channel: crate::channel_id("general"),
            text: "must never be authored".into(),
            reply_to: String::new(),
            reply,
        })
        .await
        .unwrap();
    drop(ready);
    confirmed.await.unwrap();
    assert!(
        late_result.await.is_err(),
        "no queued mutation runs after the stop boundary"
    );
    assert!(actor
        .send_reply(crate::channel_id("general"), "late", "")
        .await
        .is_err());
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn cancelling_a_queued_stop_preserves_the_running_actor() {
    let (actor, mut events, task) = start();
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let ready = park(&actor).await;
    let mut stop = Box::pin(actor.stop_and_wait());
    poll_fn(|context| {
        assert!(stop.as_mut().poll(context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(stop);
    drop(ready);
    actor
        .send_reply(crate::channel_id("general"), "still running", "")
        .await
        .unwrap();
    assert_eq!(
        actor.messages(crate::channel_id("general")).await[0].text,
        "still running"
    );
    actor.stop_and_wait().await.unwrap();
    task.await.unwrap();
    drain.await.unwrap();
}
