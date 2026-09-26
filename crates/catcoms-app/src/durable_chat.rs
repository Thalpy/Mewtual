//! A chat send is accepted only after its signed operation, MLS state, stable retry receipt
//! and publication obligation are saved together. Ready/lease custody keeps the live actor,
//! native incarnation and mounted vault ordered without queuing native locks behind commands.

use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_wire::DocType;
use tokio::sync::{oneshot, OwnedMutexGuard};

use crate::{append_message, Server, ServerStore, MAX_TRACKED_OWN_MESSAGES};

pub const MAX_SEND_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_SEND_REPLY_BYTES: usize = 256;

#[derive(Clone)]
pub struct DurableSendRequest {
    pub token: [u8; 16],
    pub expected_context: [u8; 32],
    pub channel: u128,
    pub text: String,
    pub reply_to: String,
}

impl std::fmt::Debug for DurableSendRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableSendRequest")
            .field("channel", &self.channel)
            .finish_non_exhaustive()
    }
}

impl DurableSendRequest {
    /// Bound queued plaintext before it enters the actor. Authority is checked again by the
    /// owning actor after native custody arrives. Tokens must be generated once and retained.
    pub fn validate(&self) -> Result<(), String> {
        if self.text.is_empty()
            || self.text.len() > MAX_SEND_TEXT_BYTES
            || self.reply_to.len() > MAX_SEND_REPLY_BYTES
        {
            return Err(
                "CHAT_SEND_INVALID: message or reply length is outside the supported bound".into(),
            );
        }
        Ok(())
    }

    fn binding(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/chat-send-request/v1");
        h.update(&self.expected_context);
        h.update(&self.channel.to_be_bytes());
        for field in [self.text.as_bytes(), self.reply_to.as_bytes()] {
            h.update(&(field.len() as u64).to_be_bytes());
            h.update(field);
        }
        *h.finalize().as_bytes()
    }

    fn message_id(&self, binding: &[u8; 32]) -> String {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/chat-send-id/v1");
        h.update(&self.token);
        h.update(binding);
        // Even an unsaved preparation lost in a crash keeps its user-visible identity. Such
        // an operation was never exportable; a saved preparation also retains its exact op.
        h.finalize().as_bytes()[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableSendOutcome {
    pub message_id: String,
    /// Local encrypted storage accepted the complete snapshot. This is not a peer receipt.
    pub durable: bool,
    pub replayed: bool,
    /// When present, retain the original request and token; the preparation is not accepted.
    pub storage_error: Option<String>,
}

/// Trusted native custody, obtained with try-locks only AFTER Ready. `ordering` must retain
/// the exact numeric-server incarnation, persistence serializer and session authorization.
pub struct DurableSendLease {
    store: OwnedMutexGuard<Option<ServerStore>>,
    server: u64,
    _ordering: Box<dyn Send>,
}

impl std::fmt::Debug for DurableSendLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableSendLease").finish_non_exhaustive()
    }
}

impl DurableSendLease {
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
pub struct DurableSendReady {
    lease: oneshot::Sender<DurableSendLease>,
    result: oneshot::Receiver<Result<DurableSendOutcome, String>>,
}

impl DurableSendReady {
    pub async fn commit(self, lease: DurableSendLease) -> Result<DurableSendOutcome, String> {
        self.lease
            .send(lease)
            .map_err(|_| "server stopped before chat save".to_string())?;
        // An interrupted reply is ambiguous. The native/frontend caller MUST retain the same
        // token; the saved receipt resolves whether the synchronous transaction committed.
        self.result
            .await
            .map_err(|_| "server stopped during chat save; retry the same token".to_string())?
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn commit_chat_request(
        &mut self,
        request: &DurableSendRequest,
        store: &ServerStore,
        server_id: u64,
    ) -> Result<DurableSendOutcome, String> {
        request.validate()?;
        let binding = request.binding();
        let id = request.message_id(&binding);
        let author = self.my_fingerprint();
        let ts = self.next_message_ts(request.channel);
        let prepared = self.sync.prepare_durable_chat(
            request.token,
            binding,
            request.expected_context,
            request.channel,
            id.clone(),
            |doc| append_message(doc, &id, &author, &request.text, ts, &request.reply_to),
        )?;
        let result = self
            .sync
            .commit_durable_chat(request.token, |snapshot, rng| {
                store
                    .save_server(server_id, snapshot, rng)
                    .map_err(|e| e.to_string())
            });
        let durable = result.is_ok();
        if durable {
            self.sync
                .track_delivery_target(DocType::Channel, request.channel, prepared.change);
            let recent = self.own_message_changes.entry(request.channel).or_default();
            if !recent.iter().any(|(id, _)| id == &prepared.message_id) {
                recent.push_back((prepared.message_id.clone(), prepared.change));
                while recent.len() > MAX_TRACKED_OWN_MESSAGES {
                    recent.pop_front();
                }
            }
        }
        Ok(DurableSendOutcome {
            message_id: prepared.message_id,
            durable,
            replayed: prepared.replayed,
            storage_error: result.err(),
        })
    }
}

/// Returns true when retry metadata or committed history may need an ordinary background
/// snapshot. No actor or native lock is retained while the caller emits renderer events.
pub(crate) async fn execute<T: MeshTransport, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    request: DurableSendRequest,
    ready: oneshot::Sender<DurableSendReady>,
) -> bool {
    let (lease, lease_rx) = oneshot::channel();
    let (result_tx, result) = oneshot::channel();
    if ready.send(DurableSendReady { lease, result }).is_err() {
        return false;
    }
    let Ok(lease) = lease_rx.await else {
        return false;
    };
    let result = match lease.store.as_ref() {
        Some(store) => server.commit_chat_request(&request, store, lease.server),
        None => Err("vault is not mounted".to_string()),
    };
    let touched = result.is_ok();
    drop(lease);
    let _ = result_tx.send(result);
    touched
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
    async fn durable_send_actor_failure_then_retry_and_fresh_vault_reopen_is_one_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(741);
        let store = Arc::new(Mutex::new(Some(
            ServerStore::open(dir.path(), b"test", &mut rng).unwrap(),
        )));
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(741)),
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
        let request = DurableSendRequest {
            token: [7; 16],
            expected_context: actor.durable_send_context().await.unwrap(),
            channel,
            text: "saved once despite retries".into(),
            reply_to: String::new(),
        };

        // A real store failure at its destination, not a fake success/status-return check.
        let blocked_record = dir.path().join("servers").join("9.bin");
        std::fs::create_dir(&blocked_record).unwrap();
        let pending = actor
            .prepare_durable_send(request.clone())
            .await
            .unwrap()
            .commit(DurableSendLease::new(
                store.clone().lock_owned().await,
                9,
                (),
            ))
            .await
            .unwrap();
        assert!(!pending.durable);
        assert!(pending.storage_error.is_some());
        assert!(
            actor.messages(channel).await.is_empty(),
            "unaccepted prep is not projected"
        );
        let pending_snapshot = actor.snapshot().await.unwrap();
        let hidden = Server::restore(
            &pending_snapshot,
            Hub::new().join(PeerId::from_u64(742)),
            ChaCha20Rng::seed_from_u64(742),
            Box::new(ManualClock::new(1001)),
            "alice",
        )
        .unwrap();
        assert!(
            hidden.messages(channel).is_empty(),
            "ordinary snapshots also preserve the hidden phase"
        );
        drop(hidden);
        std::fs::remove_dir(&blocked_record).unwrap();

        let accepted = actor
            .prepare_durable_send(request.clone())
            .await
            .unwrap()
            .commit(DurableSendLease::new(
                store.clone().lock_owned().await,
                9,
                (),
            ))
            .await
            .unwrap();
        assert!(accepted.durable && accepted.replayed);
        assert_eq!(pending.message_id, accepted.message_id);
        assert_eq!(actor.messages(channel).await.len(), 1);
        // Abrupt task loss: no shutdown handler or final save can make this test pass.
        task.abort();
        let _ = task.await;
        drain.await.unwrap();
        drop(store.lock().await.take());
        let reopened =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(743)).unwrap();
        let snapshot = reopened.load_server(9).unwrap();
        let restored = Server::restore(
            &snapshot,
            Hub::new().join(PeerId::from_u64(744)),
            ChaCha20Rng::seed_from_u64(744),
            Box::new(ManualClock::new(2000)),
            "alice",
        )
        .unwrap();
        assert_eq!(restored.messages(channel).len(), 1);
        assert_eq!(restored.messages(channel)[0].id, accepted.message_id);
        let (actor, mut events, task) = spawn(restored);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        *store.lock().await = Some(reopened);
        let replay = actor
            .prepare_durable_send(request.clone())
            .await
            .unwrap()
            .commit(DurableSendLease::new(
                store.clone().lock_owned().await,
                9,
                (),
            ))
            .await
            .unwrap();
        assert!(replay.durable && replay.replayed);
        assert_eq!(replay.message_id, accepted.message_id);
        let mut conflict = request.clone();
        conflict.text.push_str(" changed");
        let error = actor
            .prepare_durable_send(conflict)
            .await
            .unwrap()
            .commit(DurableSendLease::new(
                store.clone().lock_owned().await,
                9,
                (),
            ))
            .await
            .unwrap_err();
        assert!(error.starts_with("CHAT_SEND_TOKEN_CONFLICT"));
        let mut changed_basis = request;
        changed_basis.expected_context = [99; 32];
        let error = actor
            .prepare_durable_send(changed_basis)
            .await
            .unwrap()
            .commit(DurableSendLease::new(
                store.clone().lock_owned().await,
                9,
                (),
            ))
            .await
            .unwrap_err();
        assert!(error.starts_with("CHAT_SEND_TOKEN_CONFLICT"));
        assert_eq!(actor.messages(channel).await.len(), 1);
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    }

    #[tokio::test]
    async fn durable_send_dropped_ready_authors_nothing_and_does_not_hold_the_actor() {
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(745)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(745),
            Box::new(ManualClock::new(1000)),
            "alice",
        )
        .unwrap();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let channel = channel_id("general");
        actor.open_channel(channel).await;
        let request = DurableSendRequest {
            token: [8; 16],
            expected_context: actor.durable_send_context().await.unwrap(),
            channel,
            text: "cancelled before native custody".into(),
            reply_to: String::new(),
        };
        drop(actor.prepare_durable_send(request).await.unwrap());
        assert!(actor.messages(channel).await.is_empty());
        // The ordinary authoring path would reject if a hidden preparation had reserved seq1.
        actor
            .send_reply(channel, "the actor resumed", "")
            .await
            .unwrap();
        assert_eq!(actor.messages(channel).await.len(), 1);
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    }
}
