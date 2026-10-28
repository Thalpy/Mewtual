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
        // The returned row can survive later disk/UI awaits. End its exact native file scope
        // NOW under the same incarnation guard, rather than waiting for its destructor.
        servers
            .get(&self.server)
            .expect("exact incarnation checked above")
            .files
            .retire();
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
pub(crate) mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock};
    use std::future::poll_fn;

    pub(crate) fn running(
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
        entry_for(instance, server)
    }

    /// A running server whose actor has the session's encrypted disk store attached (so it can
    /// keep private Downloads) and one shared file of its own, for native tests that need a real
    /// Download record. Returns the file's content address as raw bytes.
    pub(crate) async fn running_with_file(
        instance: u64,
        state: &AppState,
        data: &[u8],
    ) -> (
        ServerEntry,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
        Vec<u8>,
    ) {
        let mut server = Server::found(
            Hub::new().join(PeerId::from_u64(instance)),
            MlsDevice::generate().unwrap(),
            OsCryptoRng,
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap();
        crate::attach_blob_store(state, &mut server).await;
        server.subscribe_control().await.unwrap();
        server.open_files().await.unwrap();
        let cid = server
            .add_file("test.bin", "application/octet-stream", "", data)
            .await
            .unwrap();
        let (entry, task, drain) = entry_for(instance, server);
        (entry, task, drain, cid.as_bytes().to_vec())
    }

    fn entry_for<T, R>(
        instance: u64,
        server: Server<T, R>,
    ) -> (
        ServerEntry,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
    )
    where
        T: MeshTransport + Send + 'static,
        R: catcoms_rt::CryptoRngCore + Send + 'static,
    {
        let group_id = server.group_id();
        let device_id = server.device_id();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        (
            ServerEntry {
                files: keep_session::EntryOwner::new(),
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
                voice: None,
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
    async fn exact_removal_retires_file_scope_before_the_returned_row_is_dropped() {
        let state = AppState::default();
        let (entry, task, drain) = running(914);
        let account = keep_session::current(&state, 0).unwrap();
        let captured = entry.files.scope(account.clone());
        entry.actor.set_keep_session(Some(captured.clone()));
        state.servers.lock().await.insert(1, entry);
        let stopped = stop_server(&state, 1, &ManualClock::new(1000))
            .await
            .unwrap()
            .unwrap();
        let removed = stopped.remove(&mut *state.servers.lock().await).unwrap();
        assert!(
            captured.is_cancelled(),
            "removed row is still retained here, but has no authority"
        );
        assert!(
            !account.is_cancelled(),
            "one server removal does not retire the whole account"
        );
        assert!(removed.files.scope(account.clone()).is_cancelled());
        let replacement = keep_session::EntryOwner::new();
        assert!(!replacement.scope(account).is_cancelled());
        assert!(captured.is_cancelled());
        drop(removed);
        task.await.unwrap();
        drain.await.unwrap();
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
