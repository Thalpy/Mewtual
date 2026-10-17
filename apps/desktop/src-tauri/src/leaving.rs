//! An actor must acknowledge its terminal boundary before native retirement of its incarnation.

use super::*;

const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// Only the completed actor handshake constructs this proof. It cannot retire a replacement.
pub(super) struct StoppedServer {
    server: u64,
    instance: u64,
}

impl StoppedServer {
    pub(super) fn remove(
        self,
        servers: &mut HashMap<u64, ServerEntry>,
    ) -> Result<ServerEntry, String> {
        if servers
            .get(&self.server)
            .is_none_or(|entry| entry.instance != self.instance)
        {
            return Err("The conversation changed while stopping; retry leaving it.".into());
        }
        Ok(servers
            .remove(&self.server)
            .expect("exact incarnation checked above"))
    }
}

pub(super) async fn stop_server(
    state: &AppState,
    server: u64,
    clock: &dyn Clock,
) -> Result<Option<StoppedServer>, String> {
    let Some((actor, instance)) = state
        .servers
        .lock()
        .await
        .get(&server)
        .map(|entry| (entry.actor.clone(), entry.instance))
    else {
        return Ok(None);
    };
    tokio::select! {
        biased;
        result = actor.stop_and_wait() => result?,
        _ = clock.sleep(STOP_DEADLINE) => {
            // Do not delete the row or bytes on an unconfirmed stop. A concurrent ack may
            // already have stopped the actor; stop_and_wait is idempotent for that retry.
            return Err("Could not confirm the conversation stopped. Its local history is kept; retry leaving it.".into());
        }
    }
    Ok(Some(StoppedServer { server, instance }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock};
    use std::future::poll_fn;

    fn running(
        instance: u64,
    ) -> (
        ServerEntry,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(instance)),
            MlsDevice::generate().unwrap(),
            OsCryptoRng,
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap();
        let group_id = server.group_id();
        let device_id = server.device_id();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        (
            ServerEntry {
                actor,
                instance,
                group_id,
                device_id,
                invite: None,
                name: "test".into(),
                bootstrap: Vec::new(),
                bootstrap_owners: HashMap::new(),
                interface_routes: None,
                rendezvous: Vec::new(),
                mesh: None,
                is_dm: false,
                switchboard: false,
                record_seq: 0,
                persist: PersistCounters::default(),
            },
            task,
            drain,
        )
    }

    #[tokio::test]
    async fn acknowledged_leave_cannot_remove_a_replacement_incarnation() {
        let state = AppState::default();
        let (old, old_task, old_drain) = running(911);
        let old_actor = old.actor.clone();
        state.servers.lock().await.insert(1, old);
        let stopped = stop_server(&state, 1, &ManualClock::new(1_000))
            .await
            .unwrap()
            .unwrap();
        let (replacement, new_task, new_drain) = running(912);
        let new_actor = replacement.actor.clone();
        state.servers.lock().await.insert(1, replacement);
        assert!(stopped.remove(&mut *state.servers.lock().await).is_err());
        assert_eq!(state.servers.lock().await[&1].instance, 912);
        assert!(old_actor
            .send_reply(channel_id("general"), "after retirement", "")
            .await
            .is_err());
        new_actor
            .send_reply(channel_id("general"), "replacement is live", "")
            .await
            .unwrap();
        let stopped = stop_server(&state, 1, &ManualClock::new(1_000))
            .await
            .unwrap()
            .unwrap();
        stopped.remove(&mut *state.servers.lock().await).unwrap();
        assert!(state.servers.lock().await.is_empty());
        old_task.await.unwrap();
        old_drain.await.unwrap();
        new_task.await.unwrap();
        new_drain.await.unwrap();
    }

    #[tokio::test]
    async fn leave_timeout_keeps_registry_and_cancels_a_still_queued_stop() {
        let state = AppState::default();
        let (entry, task, drain) = running(913);
        let actor = entry.actor.clone();
        state.servers.lock().await.insert(1, entry);
        let ready = actor
            .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
                token: [1; 16],
                expected_context: actor.durable_send_context().await.unwrap(),
                channel: channel_id("general"),
                text: "not authored".into(),
                reply_to: String::new(),
            })
            .await
            .unwrap();
        let clock = ManualClock::new(1_000);
        let mut stopping = Box::pin(stop_server(&state, 1, &clock));
        poll_fn(|context| {
            assert!(stopping.as_mut().poll(context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        clock.advance_ms(STOP_DEADLINE.as_millis() as u64);
        assert!(stopping.await.is_err());
        assert_eq!(state.servers.lock().await[&1].instance, 913);
        drop(ready);
        actor
            .send_reply(channel_id("general"), "still here", "")
            .await
            .unwrap();
        actor.stop_and_wait().await.unwrap();
        task.await.unwrap();
        drain.await.unwrap();
    }
}
