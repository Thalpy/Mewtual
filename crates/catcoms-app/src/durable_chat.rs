//! A chat send is accepted only after its signed operation, MLS state, stable retry receipt
//! and publication obligation are saved together. Ready/lease custody keeps the live actor,
//! native incarnation and mounted vault ordered without queuing native locks behind commands.

use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_wire::DocType;
use tokio::sync::{oneshot, OwnedMutexGuard};

use crate::{
    append_message, fingerprint, read_message, resolve_message, Role, Server, ServerStore,
    MAX_TRACKED_OWN_MESSAGES,
};
use automerge::{transaction::Transactable, ObjId};
use catcoms_sync::durable_chat::{ChatAction, ChatDisposition, ChatSaveError};

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
    pub disposition: ChatDisposition,
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

/// A mutation binds an immutable message object, never mutable text or edited time.
#[derive(Debug, Clone)]
pub enum DurableMutationAction {
    Edit { text: String },
    Delete,
}
#[derive(Clone)]
pub struct DurableMutationRequest {
    pub token: [u8; 16],
    pub expected_context: [u8; 32],
    pub channel: u128,
    pub message_id: String,
    pub target: [u8; 32],
    pub action: DurableMutationAction,
}
impl std::fmt::Debug for DurableMutationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableMutationRequest")
            .field("channel", &self.channel)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone)]
pub enum DurableChatRequest {
    Send(DurableSendRequest),
    Mutation(DurableMutationRequest),
}
#[derive(Debug, Clone)]
pub struct DurableStopRequest {
    pub original: DurableChatRequest,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationContext {
    pub context: [u8; 32],
    pub target: [u8; 32],
}
impl DurableMutationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.message_id.is_empty()
            || self.message_id.len() > 128
            || matches!(&self.action,DurableMutationAction::Edit{text} if text.is_empty() || text.len()>MAX_SEND_TEXT_BYTES)
        {
            return Err(
                "CHAT_MUTATION_INVALID: target or text length is outside the supported bound"
                    .into(),
            );
        }
        Ok(())
    }
    fn binding(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/chat-mutation-request/v1");
        h.update(&self.expected_context);
        h.update(&self.channel.to_be_bytes());
        h.update(&[match self.action {
            DurableMutationAction::Edit { .. } => 1,
            DurableMutationAction::Delete => 2,
        }]);
        h.update(&(self.message_id.len() as u64).to_be_bytes());
        h.update(self.message_id.as_bytes());
        h.update(&self.target);
        if let DurableMutationAction::Edit { text } = &self.action {
            h.update(&(text.len() as u64).to_be_bytes());
            h.update(text.as_bytes());
        }
        *h.finalize().as_bytes()
    }
}
impl DurableChatRequest {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Send(r) => r.validate(),
            Self::Mutation(r) => r.validate(),
        }
    }
    pub fn channel(&self) -> u128 {
        match self {
            Self::Send(r) => r.channel,
            Self::Mutation(r) => r.channel,
        }
    }
    fn identity(
        &self,
    ) -> (
        [u8; 16],
        [u8; 32],
        [u8; 32],
        u128,
        String,
        ChatAction,
        Option<[u8; 32]>,
    ) {
        match self {
            Self::Send(r) => {
                let b = r.binding();
                (
                    r.token,
                    b,
                    r.expected_context,
                    r.channel,
                    r.message_id(&b),
                    ChatAction::Send,
                    None,
                )
            }
            Self::Mutation(r) => (
                r.token,
                r.binding(),
                r.expected_context,
                r.channel,
                r.message_id.clone(),
                match r.action {
                    DurableMutationAction::Edit { .. } => ChatAction::Edit,
                    DurableMutationAction::Delete => ChatAction::Delete,
                },
                Some(r.target),
            ),
        }
    }
}
fn save_error(error: crate::AppError) -> ChatSaveError {
    match error {
        crate::AppError::CommittedButNotDurable(e) => ChatSaveError::ReplacedNotDurable(e),
        other => ChatSaveError::NotReplaced(other.to_string()),
    }
}
fn outcome(
    prepared: catcoms_sync::durable_chat::PreparedChat,
    error: Option<String>,
) -> DurableSendOutcome {
    DurableSendOutcome {
        message_id: prepared.message_id,
        durable: error.is_none() && prepared.durable,
        replayed: prepared.replayed,
        disposition: prepared.disposition,
        storage_error: error,
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Metadata only, not a capability. Resolves the exact same unique visible map as the writer.
    pub fn durable_mutation_context(
        &self,
        channel: u128,
        id: &str,
    ) -> Result<MutationContext, String> {
        let context = self.sync.durable_send_context()?;
        let doc = self
            .sync
            .doc(DocType::Channel, channel)
            .ok_or_else(|| "document not open".to_string())?;
        let location = resolve_message(doc.doc(), id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "message is missing or ambiguous".to_string())?;
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/chat-mutation-target/v1");
        let group = self.group_id();
        h.update(&(group.len() as u64).to_be_bytes());
        h.update(&group);
        h.update(&channel.to_be_bytes());
        h.update(&(id.len() as u64).to_be_bytes());
        h.update(id.as_bytes());
        // ObjId's actor-index hint and the current list index are local/mutable, not identity.
        for object in [&location.list, &location.object] {
            let ObjId::Id(counter, actor, _) = object else {
                return Err("invalid message object".into());
            };
            let bytes = actor.to_bytes();
            h.update(&(bytes.len() as u64).to_be_bytes());
            h.update(bytes);
            h.update(&counter.to_be_bytes());
        }
        Ok(MutationContext {
            context,
            target: *h.finalize().as_bytes(),
        })
    }
    fn check_mutation(&self, request: &DurableMutationRequest) -> Result<bool, String> {
        let current = self.durable_mutation_context(request.channel, &request.message_id)?;
        if current.context != request.expected_context || current.target != request.target {
            return Err("CHAT_MUTATION_TARGET_CHANGED: original target or context changed".into());
        }
        let doc = self.sync.doc(DocType::Channel, request.channel).unwrap();
        let location = resolve_message(doc.doc(), &request.message_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "message is missing or ambiguous".to_string())?;
        let message = read_message(doc.doc(), &location.object);
        let creator = doc
            .verified_object_creator(&location.object)
            .map_err(|e| e.to_string())?;
        if creator
            .as_ref()
            .is_some_and(|signer| fingerprint(signer) != message.author)
        {
            return Err(
                "CHAT_MUTATION_AUTHOR_CONFLICT: stored author contradicts verified creator".into(),
            );
        }
        let own = creator.map_or_else(
            || message.author == self.my_fingerprint(),
            |signer| signer == self.device_id,
        );
        let permitted = match request.action {
            DurableMutationAction::Edit { .. } => own,
            DurableMutationAction::Delete => {
                own || matches!(self.my_role(), Role::Owner | Role::Admin)
            }
        };
        if !permitted {
            return Err("CHAT_MUTATION_NOT_AUTHORIZED: current action permission refused".into());
        }
        Ok(matches!(&request.action,DurableMutationAction::Edit{text} if text==&message.text))
    }
    fn commit_chat_request(
        &mut self,
        request: &DurableChatRequest,
        store: &ServerStore,
        server_id: u64,
    ) -> Result<DurableSendOutcome, String> {
        request.validate()?;
        let (token, binding, context, channel, id, action, target) = request.identity();
        let existing = self
            .sync
            .durable_chat_lookup(token, binding, context, channel)?;
        if let Some(saved) = existing.as_ref().filter(|p| p.durable) {
            return Ok(outcome(saved.clone(), None));
        }
        // Uncertainty is historical repair only. Every ordinary uncommitted retry still checks
        // current membership, exact target and role, including same-epoch admin demotion.
        let needs_repair = self.sync.durable_chat_token_needs_repair(token);
        if existing.is_none() || !needs_repair {
            match request {
                DurableChatRequest::Send(r) => {
                    let author = self.my_fingerprint();
                    let ts = self.next_message_ts(channel);
                    self.sync.prepare_durable_chat(
                        token,
                        binding,
                        context,
                        channel,
                        id.clone(),
                        |doc| append_message(doc, &id, &author, &r.text, ts, &r.reply_to),
                    )?;
                }
                DurableChatRequest::Mutation(r) => {
                    let noop = self.check_mutation(r)?;
                    let edited = self.sync.now_ms();
                    self.sync.prepare_durable_chat_action(
                        token,
                        binding,
                        context,
                        channel,
                        id.clone(),
                        action,
                        target,
                        noop,
                        |doc| {
                            let location = resolve_message(doc, &id)?.ok_or_else(|| {
                                automerge::AutomergeError::InvalidObjId(id.clone())
                            })?;
                            match &r.action {
                                DurableMutationAction::Edit { text } => {
                                    doc.put(&location.object, crate::TEXT, text.as_str())?;
                                    doc.put(&location.object, crate::EDITED, edited as i64)?;
                                }
                                DurableMutationAction::Delete => {
                                    doc.delete(&location.list, location.index)?
                                }
                            }
                            Ok(())
                        },
                    )?;
                }
            }
        }
        let result = self.sync.commit_durable_chat(token, |bytes, rng| {
            store.save_server(server_id, bytes, rng).map_err(save_error)
        });
        let mut prepared = self
            .sync
            .durable_chat_lookup(token, binding, context, channel)?
            .ok_or_else(|| "chat preparation disappeared".to_string())?;
        prepared.replayed = existing.is_some();
        if result.is_ok() && action == ChatAction::Send {
            if let Some(change) = prepared.change {
                self.sync
                    .track_delivery_target(DocType::Channel, channel, change);
                let recent = self.own_message_changes.entry(channel).or_default();
                if !recent.iter().any(|(id, _)| id == &prepared.message_id) {
                    recent.push_back((prepared.message_id.clone(), change));
                    while recent.len() > MAX_TRACKED_OWN_MESSAGES {
                        recent.pop_front();
                    }
                }
            }
        }
        Ok(outcome(prepared, result.err().map(|e| e.to_string())))
    }
    fn stop_chat_request(
        &mut self,
        request: &DurableStopRequest,
        store: &ServerStore,
        server_id: u64,
    ) -> Result<DurableSendOutcome, String> {
        request.original.validate()?;
        let (token, binding, context, channel, id, action, target) = request.original.identity();
        let before = self
            .sync
            .durable_chat_lookup(token, binding, context, channel)?;
        match self.sync.stop_durable_chat(
            token,
            binding,
            context,
            channel,
            id.clone(),
            action,
            target,
            |bytes, rng| store.save_server(server_id, bytes, rng).map_err(save_error),
        ) {
            Ok(saved) => Ok(outcome(saved, None)),
            Err(error) => {
                let possible = self
                    .sync
                    .durable_chat_lookup(token, binding, context, channel)?
                    .or(before);
                Ok(DurableSendOutcome {
                    message_id: id,
                    durable: false,
                    replayed: possible.as_ref().is_some_and(|p| p.replayed),
                    disposition: possible
                        .map_or(ChatDisposition::StoppedBeforeCommit, |p| p.disposition),
                    storage_error: Some(error.to_string()),
                })
            }
        }
    }
}
/// No native guard is queued behind Actor commands. Ready owns the synchronous transaction
/// only after the host transfers its exact mounted-store/incarnation lease.
pub(crate) async fn execute<T: MeshTransport, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    request: DurableChatRequest,
    stop: bool,
    ready: oneshot::Sender<DurableSendReady>,
) -> bool {
    let (lease, rx) = oneshot::channel();
    let (tx, result) = oneshot::channel();
    if ready.send(DurableSendReady { lease, result }).is_err() {
        return false;
    }
    let Ok(lease) = rx.await else {
        return false;
    };
    let result = match lease.store.as_ref() {
        Some(store) => {
            if stop {
                server.stop_chat_request(
                    &DurableStopRequest { original: request },
                    store,
                    lease.server,
                )
            } else {
                server.commit_chat_request(&request, store, lease.server)
            }
        }
        None => Err("vault is not mounted".into()),
    };
    let touched = result.is_ok();
    drop(lease);
    let _ = tx.send(result);
    touched
}
#[derive(Debug)]
pub struct DurableRepairReady {
    lease: oneshot::Sender<DurableSendLease>,
    result: oneshot::Receiver<Result<(), String>>,
}
impl DurableRepairReady {
    pub async fn commit(self, lease: DurableSendLease) -> Result<(), String> {
        self.lease
            .send(lease)
            .map_err(|_| "server stopped before repair".to_string())?;
        self.result
            .await
            .map_err(|_| "server stopped during repair".to_string())?
    }
}
pub(crate) async fn execute_repair<T: MeshTransport, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    ready: oneshot::Sender<Option<DurableRepairReady>>,
) -> bool {
    if !server.sync.durable_chat_needs_repair() {
        let _ = ready.send(None);
        return false;
    }
    let (lease, rx) = oneshot::channel();
    let (tx, result) = oneshot::channel();
    if ready
        .send(Some(DurableRepairReady { lease, result }))
        .is_err()
    {
        return false;
    }
    let Ok(lease) = rx.await else {
        return false;
    };
    let result = match lease.store.as_ref() {
        Some(store) => server
            .sync
            .repair_durable_chat(|bytes, rng| {
                store
                    .save_server(lease.server, bytes, rng)
                    .map_err(save_error)
            })
            .map_err(|e| e.to_string()),
        None => Err("vault is not mounted".into()),
    };
    let touched = result.is_ok();
    drop(lease);
    let _ = tx.send(result);
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

    type Node = Server<catcoms_rt::MemNetwork, ChaCha20Rng>;
    async fn mutation_node() -> Node {
        let mut n = Server::found(
            Hub::new().join(PeerId::from_u64(881)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(881),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        n.open_channel(5).await.unwrap();
        n
    }
    fn send_request(n: &Node, token: u8, text: &str) -> DurableChatRequest {
        DurableChatRequest::Send(DurableSendRequest {
            token: [token; 16],
            expected_context: n.sync.durable_send_context().unwrap(),
            channel: 5,
            text: text.into(),
            reply_to: String::new(),
        })
    }
    fn mutation_request(
        n: &Node,
        token: u8,
        id: &str,
        action: DurableMutationAction,
    ) -> DurableChatRequest {
        let target = n.durable_mutation_context(5, id).unwrap();
        DurableChatRequest::Mutation(DurableMutationRequest {
            token: [token; 16],
            expected_context: target.context,
            channel: 5,
            message_id: id.into(),
            target: target.target,
            action,
        })
    }
    fn restored(n: &ServerStore, id: u64) -> Node {
        Server::restore(
            &n.load_server(id).unwrap(),
            Hub::new().join(PeerId::from_u64(882)),
            ChaCha20Rng::seed_from_u64(882),
            Box::new(ManualClock::new(2_000)),
            "alice",
        )
        .unwrap()
    }
    fn ops(n: &Node) -> usize {
        n.sync.doc(DocType::Channel, 5).unwrap().op_count()
    }

    #[tokio::test]
    async fn durable_mutation_edit_delete_and_noop_reopen_exact_original_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(883)).unwrap();
        let mut n = mutation_node().await;
        let post = n
            .commit_chat_request(&send_request(&n, 1, "original"), &store, 9)
            .unwrap();
        assert!(post.durable && !post.replayed);
        let edit = mutation_request(
            &n,
            2,
            &post.message_id,
            DurableMutationAction::Edit {
                text: "edited".into(),
            },
        );
        let edited = n.commit_chat_request(&edit, &store, 9).unwrap();
        assert!(edited.durable && !edited.replayed);
        assert_eq!(ops(&n), 2);
        let noop = mutation_request(
            &n,
            3,
            &post.message_id,
            DurableMutationAction::Edit {
                text: "edited".into(),
            },
        );
        let before = n.messages(5)[0].edited;
        assert!(n.commit_chat_request(&noop, &store, 9).unwrap().durable);
        assert_eq!(ops(&n), 2);
        assert_eq!(n.messages(5)[0].edited, before);
        let mut reopened = restored(&store, 9);
        assert!(
            reopened
                .commit_chat_request(&edit, &store, 9)
                .unwrap()
                .replayed
        );
        assert_eq!(ops(&reopened), 2);
        let delete = mutation_request(
            &reopened,
            4,
            &post.message_id,
            DurableMutationAction::Delete,
        );
        assert!(
            reopened
                .commit_chat_request(&delete, &store, 9)
                .unwrap()
                .durable
        );
        assert!(reopened.messages(5).is_empty());
        assert_eq!(ops(&reopened), 3);
        let mut reopened = restored(&store, 9);
        assert!(
            reopened
                .commit_chat_request(&delete, &store, 9)
                .unwrap()
                .replayed
        );
        assert!(
            reopened
                .commit_chat_request(&noop, &store, 9)
                .unwrap()
                .durable
        );
        assert_eq!(ops(&reopened), 3);
        let DurableChatRequest::Mutation(mut foreign) = edit else {
            unreachable!()
        };
        foreign.target = [99; 32];
        assert!(reopened
            .commit_chat_request(&DurableChatRequest::Mutation(foreign), &store, 9)
            .unwrap_err()
            .starts_with("CHAT_SEND_TOKEN_CONFLICT"));
    }

    #[tokio::test]
    async fn durable_mutation_real_pre_rename_failure_keeps_private_exact_edit() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(884)).unwrap();
        let mut n = mutation_node().await;
        let post = n
            .commit_chat_request(&send_request(&n, 1, "original"), &store, 9)
            .unwrap();
        let edit = mutation_request(
            &n,
            2,
            &post.message_id,
            DurableMutationAction::Edit {
                text: "private".into(),
            },
        );
        let blocked = dir.path().join("servers/10.bin");
        std::fs::create_dir(&blocked).unwrap();
        let pending = n.commit_chat_request(&edit, &store, 10).unwrap();
        assert!(!pending.durable);
        assert_eq!(n.messages(5)[0].text, "original");
        assert_eq!(ops(&n), 1);
        assert!(!n.sync.durable_chat_needs_repair());
        assert!(n.snapshot().is_ok());
        std::fs::remove_dir(blocked).unwrap();
        let committed = n.commit_chat_request(&edit, &store, 10).unwrap();
        assert!(committed.durable && committed.replayed);
        assert_eq!(ops(&n), 2);
        let reopened = restored(&store, 10);
        assert_eq!(reopened.messages(5)[0].text, "private");
        assert_eq!(ops(&reopened), 2);
    }

    #[tokio::test]
    async fn durable_mutation_actual_rename_uncertainty_blocks_snapshots_and_repairs_original() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(885)).unwrap();
        let mut n = mutation_node().await;
        let post = n
            .commit_chat_request(&send_request(&n, 1, "original"), &store, 9)
            .unwrap();
        let edit = mutation_request(
            &n,
            2,
            &post.message_id,
            DurableMutationAction::Edit {
                text: "renamed".into(),
            },
        );
        store.fail_next_chat_parent_sync();
        let pending = n.commit_chat_request(&edit, &store, 9).unwrap();
        assert!(!pending.durable);
        assert!(pending.storage_error.unwrap().contains("actual rename"));
        assert_eq!(n.messages(5)[0].text, "original");
        assert_eq!(ops(&n), 1);
        assert!(
            n.snapshot().is_err(),
            "ordinary rollback snapshot must not overwrite possible saved history"
        );
        let on_disk = restored(&store, 9);
        assert_eq!(on_disk.messages(5)[0].text, "renamed");
        assert_eq!(ops(&on_disk), 2);
        n.sync
            .repair_durable_chat(|bytes, rng| store.save_server(9, bytes, rng).map_err(save_error))
            .unwrap();
        assert_eq!(n.messages(5)[0].text, "renamed");
        assert_eq!(ops(&n), 2);
        assert!(n.snapshot().is_ok());
        assert!(n.commit_chat_request(&edit, &store, 9).unwrap().replayed);
        assert_eq!(ops(&n), 2);
        assert_eq!(ops(&restored(&store, 9)), 2);
    }

    #[tokio::test]
    async fn durable_mutation_real_noop_and_unknown_stop_rename_failures_latch_whole_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(886)).unwrap();
        let mut n = mutation_node().await;
        let post = n
            .commit_chat_request(&send_request(&n, 1, "same"), &store, 9)
            .unwrap();
        let noop = mutation_request(
            &n,
            2,
            &post.message_id,
            DurableMutationAction::Edit {
                text: "same".into(),
            },
        );
        store.fail_next_chat_parent_sync();
        assert!(!n.commit_chat_request(&noop, &store, 9).unwrap().durable);
        assert!(n.snapshot().is_err());
        assert_eq!(ops(&restored(&store, 9)), 1);
        n.sync
            .repair_durable_chat(|bytes, rng| store.save_server(9, bytes, rng).map_err(save_error))
            .unwrap();
        assert!(n.commit_chat_request(&noop, &store, 9).unwrap().durable);
        assert_eq!(ops(&n), 1);
        let late = send_request(&n, 3, "must never be authored");
        store.fail_next_chat_parent_sync();
        let stopped = n
            .stop_chat_request(
                &DurableStopRequest {
                    original: late.clone(),
                },
                &store,
                9,
            )
            .unwrap();
        assert!(!stopped.durable);
        assert_eq!(stopped.disposition, ChatDisposition::StoppedBeforeCommit);
        assert!(n.snapshot().is_err());
        let mut disk = restored(&store, 9);
        let saved = disk.commit_chat_request(&late, &store, 9).unwrap();
        assert!(saved.durable);
        assert_eq!(saved.disposition, ChatDisposition::StoppedBeforeCommit);
        assert_eq!(ops(&disk), 1);
        let saved = n.commit_chat_request(&late, &store, 9).unwrap();
        assert!(saved.durable);
        assert_eq!(saved.disposition, ChatDisposition::StoppedBeforeCommit);
        assert_eq!(ops(&n), 1);
    }

    #[tokio::test]
    async fn durable_mutation_stop_after_commit_preserves_saved_history_and_exact_token() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(887)).unwrap();
        let mut n = mutation_node().await;
        let send = send_request(&n, 1, "retained history");
        assert!(n.commit_chat_request(&send, &store, 9).unwrap().durable);
        store.fail_next_chat_parent_sync();
        let stopped = n
            .stop_chat_request(
                &DurableStopRequest {
                    original: send.clone(),
                },
                &store,
                9,
            )
            .unwrap();
        assert!(!stopped.durable);
        assert_eq!(stopped.disposition, ChatDisposition::StoppedAfterCommit);
        assert_eq!(ops(&n), 1);
        assert_eq!(ops(&restored(&store, 9)), 1);
        assert!(n.snapshot().is_err());
        let stopped = n
            .stop_chat_request(
                &DurableStopRequest {
                    original: send.clone(),
                },
                &store,
                9,
            )
            .unwrap();
        assert!(stopped.durable);
        assert_eq!(stopped.disposition, ChatDisposition::StoppedAfterCommit);
        let replay = n.commit_chat_request(&send, &store, 9).unwrap();
        assert!(replay.durable);
        assert_eq!(replay.disposition, ChatDisposition::StoppedAfterCommit);
        assert_eq!(ops(&n), 1);
    }

    #[tokio::test]
    async fn durable_mutation_target_ignores_text_but_rejects_substitution_and_duplicate_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(888)).unwrap();
        let mut n = mutation_node().await;
        let posted = n
            .commit_chat_request(&send_request(&n, 1, "original"), &store, 9)
            .unwrap();
        let edit = mutation_request(
            &n,
            2,
            &posted.message_id,
            DurableMutationAction::Edit {
                text: "retained intent".into(),
            },
        );
        n.edit_message(5, &posted.message_id, "intervening edit")
            .await
            .unwrap();
        assert!(
            n.commit_chat_request(&edit, &store, 9).unwrap().durable,
            "same object remains addressed across text edits"
        );
        let stale = mutation_request(&n, 3, &posted.message_id, DurableMutationAction::Delete);
        let id = posted.message_id.clone();
        let author = n.my_fingerprint();
        n.sync
            .post(DocType::Channel, 5, |doc| {
                crate::delete_message_in_doc(doc, &id)?;
                append_message(doc, &id, &author, "replacement", 2_000, "")
            })
            .await
            .unwrap();
        assert!(n
            .commit_chat_request(&stale, &store, 9)
            .unwrap_err()
            .starts_with("CHAT_MUTATION_TARGET_CHANGED"));
        let ambiguous = mutation_request(&n, 4, &posted.message_id, DurableMutationAction::Delete);
        n.sync
            .post(DocType::Channel, 5, |doc| {
                append_message(doc, &id, &author, "duplicate", 2_001, "")
            })
            .await
            .unwrap();
        assert!(n
            .commit_chat_request(&ambiguous, &store, 9)
            .unwrap_err()
            .contains("ambiguous"));
        assert_eq!(n.messages(5).len(), 2);
    }
    #[tokio::test]
    async fn durable_mutation_verified_creator_cannot_be_relabelled_by_author_field() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ServerStore::open(dir.path(), b"test", &mut ChaCha20Rng::seed_from_u64(889)).unwrap();
        let mut n = mutation_node().await;
        let post = n
            .commit_chat_request(&send_request(&n, 1, "real author"), &store, 9)
            .unwrap();
        n.sync
            .post(DocType::Channel, 5, |doc| {
                let location = resolve_message(doc, &post.message_id)?.unwrap();
                doc.put(&location.object, crate::AUTHOR, "another display author")
            })
            .await
            .unwrap();
        let request = mutation_request(&n, 2, &post.message_id, DurableMutationAction::Delete);
        assert!(n
            .commit_chat_request(&request, &store, 9)
            .unwrap_err()
            .starts_with("CHAT_MUTATION_AUTHOR_CONFLICT"));
        assert_eq!(n.messages(5).len(), 1);
    }
}
