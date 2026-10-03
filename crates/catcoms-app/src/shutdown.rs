//! Save and freeze an actor for orderly application shutdown.
//!
//! Acquiring custody follows the same Ready/lease contract as other store transactions. A
//! frozen actor accepts no more work until the caller either commits shutdown or drops the
//! handle to resume it. This lets a multi-group close fail without leaving half its actors dead.

use catcoms_rt::{CryptoRngCore, MeshTransport};
use tokio::sync::{oneshot, OwnedMutexGuard};

use crate::{Server, ServerStore};

/// Trusted native custody. Obtain all ordering guards with try-locks AFTER the actor is Ready.
/// The ordering value must fence the numeric server, registry incarnation and UI session.
pub struct ShutdownLease {
    store: OwnedMutexGuard<Option<ServerStore>>,
    server: u64,
    _ordering: Box<dyn Send>,
}

impl std::fmt::Debug for ShutdownLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShutdownLease")
            .finish_non_exhaustive()
    }
}

impl ShutdownLease {
    pub fn new(
        store: OwnedMutexGuard<Option<ServerStore>>,
        server: u64,
        ordering: impl Send + 'static,
    ) -> Self {
        Self {
            store,
            server,
            _ordering: Box::new(ordering),
        }
    }
}

#[derive(Debug)]
pub struct ShutdownReady {
    lease: oneshot::Sender<ShutdownLease>,
    result: oneshot::Receiver<Result<FrozenServer, String>>,
}

impl ShutdownReady {
    pub async fn save_and_freeze(self, lease: ShutdownLease) -> Result<FrozenServer, String> {
        self.lease
            .send(lease)
            .map_err(|_| "server stopped before shutdown save".to_string())?;
        self.result
            .await
            .map_err(|_| "server stopped during shutdown save".to_string())?
    }
}

/// Dropping this handle resumes the actor. Only `stop` ends an already durably saved actor.
#[derive(Debug)]
pub struct FrozenServer {
    release: oneshot::Sender<bool>,
}

impl FrozenServer {
    pub fn stop(self) {
        let _ = self.release.send(true);
    }
}

pub(crate) async fn save_and_freeze<T: MeshTransport, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    ready: oneshot::Sender<ShutdownReady>,
) -> bool {
    let (lease_tx, lease_rx) = oneshot::channel();
    let (result_tx, result_rx) = oneshot::channel();
    if ready
        .send(ShutdownReady {
            lease: lease_tx,
            result: result_rx,
        })
        .is_err()
    {
        return false;
    }
    let Ok(lease) = lease_rx.await else {
        return false;
    };
    let result = (|| {
        let store = lease
            .store
            .as_ref()
            .ok_or_else(|| "vault is not mounted".to_string())?;
        let snapshot = server.snapshot().map_err(|error| error.to_string())?;
        server
            .sync
            .with_registry_context(|_, _, _, rng| store.save_server(lease.server, &snapshot, rng))
            .map_err(|error| error.to_string())
    })();
    // Never retain vault/native locks while the event consumer or close coordinator awaits us.
    drop(lease);
    if let Err(error) = result {
        let _ = result_tx.send(Err(error));
        return false;
    }
    let (release, decision) = oneshot::channel();
    if result_tx.send(Ok(FrozenServer { release })).is_err() {
        return false;
    }
    matches!(decision.await, Ok(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{channel_id, spawn};
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn shutdown_barrier_saves_before_freezing_and_drop_resumes_the_actor() {
        let root = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(891);
        let vault = Arc::new(Mutex::new(Some(
            ServerStore::open(root.path(), b"test", &mut rng).unwrap(),
        )));
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(891)),
            MlsDevice::generate().unwrap(),
            rng,
            Box::new(ManualClock::new(1000)),
            "alice",
        )
        .unwrap();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let channel = channel_id("general");
        actor.open_channel(channel).await;
        actor
            .send_reply(channel, "before the close barrier", "")
            .await
            .unwrap();

        let ready = actor.prepare_shutdown().await.unwrap();
        let frozen = ready
            .save_and_freeze(ShutdownLease::new(vault.clone().lock_owned().await, 9, ()))
            .await
            .unwrap();
        let first = vault.lock().await.as_ref().unwrap().load_server(9).unwrap();
        let restored = Server::restore(
            &first,
            Hub::new().join(PeerId::from_u64(892)),
            ChaCha20Rng::seed_from_u64(892),
            Box::new(ManualClock::new(2000)),
            "alice",
        )
        .unwrap();
        assert_eq!(
            restored.messages(channel)[0].text,
            "before the close barrier"
        );

        // Cancelling a later group's close must make this actor usable again.
        drop(frozen);
        actor
            .send_reply(channel, "after cancelled close", "")
            .await
            .unwrap();
        let ready = actor.prepare_shutdown().await.unwrap();
        let frozen = ready
            .save_and_freeze(ShutdownLease::new(vault.clone().lock_owned().await, 9, ()))
            .await
            .unwrap();
        frozen.stop();
        task.await.unwrap();
        drain.await.unwrap();
        drop(vault.lock().await.take());
        let reopened =
            ServerStore::open(root.path(), b"test", &mut ChaCha20Rng::seed_from_u64(893)).unwrap();
        let final_snapshot = reopened.load_server(9).unwrap();
        let restored = Server::restore(
            &final_snapshot,
            Hub::new().join(PeerId::from_u64(894)),
            ChaCha20Rng::seed_from_u64(894),
            Box::new(ManualClock::new(3000)),
            "alice",
        )
        .unwrap();
        assert_eq!(restored.messages(channel).len(), 2);
    }

    #[tokio::test]
    async fn shutdown_refuses_unmounted_storage_and_dropped_ready_does_not_stop_actor() {
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(895)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(895),
            Box::new(ManualClock::new(1000)),
            "alice",
        )
        .unwrap();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        drop(actor.prepare_shutdown().await.unwrap());
        let ready = actor.prepare_shutdown().await.unwrap();
        let empty = Arc::new(Mutex::new(None));
        assert!(ready
            .save_and_freeze(ShutdownLease::new(empty.lock_owned().await, 9, ()))
            .await
            .is_err());
        assert!(actor.snapshot().await.is_ok());
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    }
}
