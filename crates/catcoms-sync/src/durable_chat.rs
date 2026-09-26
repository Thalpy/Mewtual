//! Local chat preparation and durable publication obligations.
//!
//! A prepared operation is deliberately absent from `docs`: gossip AND request-based history
//! export therefore see only committed history. The storage callback below is synchronous and
//! runs under the actor's exclusive ownership. There is no network await between constructing
//! the candidate snapshot, saving it, and installing the committed document.

use super::*;

/// Receipts are never evicted: forgetting a successful token could turn a retry into a new op.
pub const MAX_CHAT_RECEIPTS: usize = 65_536;
/// Includes both uncommitted preparations and committed publications awaiting submission.
pub const MAX_CHAT_PENDING: usize = 32;
const MAX_SEALED_CHAT: usize = 256 * 1024;
const PUBLICATION_RETRY_MS: u64 = 1_000;
const SNAPSHOT_VERSION: u8 = 1;

#[derive(Default)]
pub(crate) struct DurableChatState {
    records: BTreeMap<[u8; 16], ChatRecord>,
    // Rebuilt on restore. Hot actor turns touch at most MAX_CHAT_PENDING entries.
    active: BTreeSet<[u8; 16]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::{transaction::Transactable, ROOT};
    use catcoms_rt::ManualClock;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };

    #[derive(Debug, Default)]
    struct RecordingTransport {
        published: Mutex<Vec<Vec<u8>>>,
        refuse: AtomicBool,
        duplicate: AtomicBool,
    }

    #[async_trait::async_trait]
    impl MeshTransport for RecordingTransport {
        fn local_peer(&self) -> PeerId {
            PeerId::from_u64(41)
        }
        async fn subscribe(&self, _: Topic) -> Result<(), TransportError> {
            Ok(())
        }
        async fn unsubscribe(&self, _: Topic) -> Result<(), TransportError> {
            Ok(())
        }
        async fn publish(&self, _: Topic, bytes: Bytes) -> Result<(), TransportError> {
            if self.refuse.load(Ordering::SeqCst) {
                return Err(TransportError::Closed);
            }
            self.published.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
        async fn publish_once(
            &self,
            topic: Topic,
            bytes: Bytes,
        ) -> Result<catcoms_rt::PublishSubmission, catcoms_rt::PublishOnceError> {
            if self.duplicate.load(Ordering::SeqCst) {
                return Ok(catcoms_rt::PublishSubmission::Duplicate);
            }
            self.publish(topic, bytes)
                .await
                .map_err(|_| catcoms_rt::PublishOnceError::Closed)?;
            Ok(catcoms_rt::PublishSubmission::Submitted)
        }
        async fn request(
            &self,
            _: PeerId,
            _: ProtocolId,
            _: Bytes,
        ) -> Result<Bytes, TransportError> {
            Err(TransportError::Closed)
        }
        async fn request_cancellable(
            &self,
            peer: PeerId,
            protocol: ProtocolId,
            bytes: Bytes,
            _: catcoms_rt::RequestCancellation,
        ) -> Result<Bytes, TransportError> {
            self.request(peer, protocol, bytes).await
        }
        async fn next_event(&self) -> Option<TransportEvent> {
            std::future::pending().await
        }
    }

    type Node = ChannelSync<RecordingTransport, ChaCha20Rng>;

    fn node() -> Node {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        ChannelSync::new(
            RecordingTransport::default(),
            group,
            device,
            ChaCha20Rng::seed_from_u64(41),
            Box::new(ManualClock::new(1_000)),
        )
    }

    fn restore(snapshot: &[u8]) -> Node {
        ChannelSync::restore(
            snapshot,
            RecordingTransport::default(),
            ChaCha20Rng::seed_from_u64(42),
            Box::new(ManualClock::new(2_000)),
        )
        .unwrap()
    }

    fn prepare(node: &mut Node, token: u8, channel: u128) -> PreparedChat {
        let context = node.durable_send_context().unwrap();
        node.prepare_durable_chat(
            [token; 16],
            [token; 32],
            context,
            channel,
            format!("m{token}"),
            |d| d.put(ROOT, format!("m{token}"), "message"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn durable_chat_failed_save_never_exports_and_retry_keeps_exact_operation() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        let first = prepare(&mut n, 1, 5);
        let sealed = n.durable_chat.records[&[1; 16]].sealed.clone();
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 0);
        let error = n
            .commit_durable_chat([1; 16], |snapshot, _| {
                let restored = restore(snapshot);
                assert_eq!(restored.doc(DocType::Channel, 5).unwrap().op_count(), 1);
                assert!(restored.durable_chat.records[&[1; 16]].durable);
                assert!(!restored.durable_chat.records[&[1; 16]].sealed.is_empty());
                Err("injected disk failure".into())
            })
            .unwrap_err();
        assert_eq!(error, "injected disk failure");
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 0);
        let exported = n.docs[&(DocType::Channel, 5)]
            .export_catchup(&n.group, &n.device, &mut n.rng)
            .unwrap();
        assert!(
            exported.is_empty(),
            "request-based serving uses the same committed log"
        );
        n.drain_durable_chat().await;
        assert!(n.transport.published.lock().unwrap().is_empty());
        assert!(matches!(
            n.post(DocType::Channel, 5, |d| d.put(ROOT, "other", "collision"))
                .await,
            Err(SyncError::ChatPreparationPending)
        ));
        let second = prepare(&mut n, 1, 5);
        assert!(second.replayed);
        assert_eq!(first.change, second.change);
        assert_eq!(n.durable_chat.records[&[1; 16]].sealed, sealed);
        n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
        n.drain_durable_chat().await;
        assert_eq!(*n.transport.published.lock().unwrap(), vec![sealed]);
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 1);
    }

    #[tokio::test]
    async fn durable_chat_restores_preparation_and_publication_without_clean_shutdown() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        let original = prepare(&mut n, 2, 5);
        let sealed = n.durable_chat.records[&[2; 16]].sealed.clone();
        let prepared_snapshot = n.snapshot().unwrap();
        let mut n = restore(&prepared_snapshot);
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 0);
        let context = n.durable_send_context().unwrap();
        let replay = n
            .prepare_durable_chat([2; 16], [2; 32], context, 5, "ignored".into(), |_| {
                panic!("retry must not edit or re-sign")
            })
            .unwrap();
        assert_eq!(replay.change, original.change);
        let mut durable_snapshot = Vec::new();
        n.commit_durable_chat([2; 16], |snapshot, _| {
            durable_snapshot = snapshot.to_vec();
            Ok(())
        })
        .unwrap();
        // Model abrupt process loss immediately after the save, before publication or a reply.
        drop(n);
        let clock = ManualClock::new(2_000);
        let mut recovered = ChannelSync::restore(
            &durable_snapshot,
            RecordingTransport::default(),
            ChaCha20Rng::seed_from_u64(42),
            Box::new(clock.clone()),
        )
        .unwrap();
        assert_eq!(recovered.doc(DocType::Channel, 5).unwrap().op_count(), 1);
        let replay = prepare(&mut recovered, 2, 5);
        assert!(replay.durable && replay.replayed);
        recovered.transport.refuse.store(true, Ordering::SeqCst);
        recovered.drain_durable_chat().await;
        assert_eq!(recovered.durable_chat.records[&[2; 16]].sealed, sealed);
        clock.set_wall_ms(1);
        assert_eq!(
            recovered.next_durable_chat_retry_delay(),
            Some(PUBLICATION_RETRY_MS),
            "wall-clock correction must not defer the process-local retry"
        );
        recovered.transport.refuse.store(false, Ordering::SeqCst);
        clock.advance_ms(PUBLICATION_RETRY_MS);
        recovered.transport.duplicate.store(true, Ordering::SeqCst);
        recovered.drain_durable_chat().await;
        assert_eq!(
            recovered.durable_chat.records[&[2; 16]].sealed, sealed,
            "cache duplicate is not proof of a prior driver submission"
        );
        recovered.transport.duplicate.store(false, Ordering::SeqCst);
        clock.advance_ms(PUBLICATION_RETRY_MS);
        // Exercise the production run_once hook, stopping once it waits for an inbound event.
        use futures::FutureExt;
        assert!(recovered.run_once().now_or_never().is_none());
        assert_eq!(*recovered.transport.published.lock().unwrap(), vec![sealed]);
        assert!(recovered.durable_chat.records[&[2; 16]].sealed.is_empty());
    }

    #[tokio::test]
    async fn durable_chat_context_drift_retains_obligation_and_receipt_but_blocks_new_authoring() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        n.open_channel(DocType::Channel, 6).await.unwrap();
        let context = n.durable_send_context().unwrap();
        prepare(&mut n, 1, 5);
        n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
        prepare(&mut n, 2, 6);
        let peer = MlsDevice::generate().unwrap();
        n.with_observed_mls_transition(|node| {
            node.group
                .add_member(&node.device, peer.key_package().unwrap())
        })
        .unwrap();
        assert_ne!(context, n.durable_send_context().unwrap());
        let replay = n
            .prepare_durable_chat([1; 16], [1; 32], context, 5, "ignored".into(), |_| panic!())
            .unwrap();
        assert!(replay.durable && replay.replayed);
        assert!(n
            .prepare_durable_chat([1; 16], [9; 32], context, 5, "ignored".into(), |_| panic!())
            .unwrap_err()
            .starts_with("CHAT_SEND_TOKEN_CONFLICT"));
        for (token, channel) in [(2, 6), (3, 5)] {
            assert!(n
                .prepare_durable_chat(
                    [token; 16],
                    [token; 32],
                    context,
                    channel,
                    "ignored".into(),
                    |_| panic!()
                )
                .unwrap_err()
                .starts_with("CHAT_SEND_CONTEXT_CHANGED"));
        }
        n.drain_durable_chat().await;
        assert!(n.transport.published.lock().unwrap().is_empty());
        assert!(n.durable_chat.records[&[1; 16]].stopped);
        assert!(n.durable_chat.records[&[1; 16]].sealed.is_empty());
        assert!(!n.durable_chat.records[&[2; 16]].durable);
        assert!(n.durable_chat.records[&[2; 16]].stopped);
        // A fresh explicit decision under current authority must not leave this channel
        // permanently wedged behind a preparation that can never be authorized again.
        let fresh = prepare(&mut n, 3, 6);
        n.commit_durable_chat([3; 16], |_, _| Ok(())).unwrap();
        assert!(!fresh.replayed);
        assert_eq!(n.doc(DocType::Channel, 6).unwrap().op_count(), 1);
        let snap = n.snapshot().unwrap();
        let mut restored = restore(&snap);
        assert!(restored
            .prepare_durable_chat([2; 16], [2; 32], context, 6, "old".into(), |_| panic!())
            .unwrap_err()
            .starts_with("CHAT_SEND_CONTEXT_CHANGED"));
    }

    #[tokio::test]
    async fn durable_chat_pending_capacity_refuses_new_work_without_eviction() {
        let mut n = node();
        for token in 0..MAX_CHAT_PENDING as u8 {
            n.open_channel(DocType::Channel, u128::from(token))
                .await
                .unwrap();
            prepare(&mut n, token, u128::from(token));
        }
        n.open_channel(DocType::Channel, 99).await.unwrap();
        let context = n.durable_send_context().unwrap();
        assert!(n
            .prepare_durable_chat([99; 16], [99; 32], context, 99, "new".into(), |_| panic!())
            .unwrap_err()
            .starts_with("CHAT_SEND_CAPACITY"));
        assert_eq!(n.durable_chat.records.len(), MAX_CHAT_PENDING);
        assert!(prepare(&mut n, 0, 0).replayed);
        let encoded = n.durable_chat.encode().unwrap();
        assert_eq!(
            DurableChatState::decode(&encoded).unwrap().records.len(),
            MAX_CHAT_PENDING
        );
        let mut unknown = encoded;
        unknown[0] = 255;
        assert!(DurableChatState::decode(&unknown).is_err());
    }
}

struct ChatRecord {
    binding: [u8; 32],
    context: [u8; 32],
    channel: u128,
    message_id: String,
    change: ChangeHash,
    durable: bool,
    // Authority moved: this token stays bound but its preparation/publication is terminal.
    // A committed operation remains ordinary signed history; an uncommitted one never was.
    stopped: bool,
    // Exact original sealed signed op, never re-signed/re-sealed on retry. Cleared only after
    // successful transport submission; the signed log remains the canonical chat history.
    sealed: Vec<u8>,
    // Process-local retry cadence. Restart permits an immediate attempt of a saved obligation.
    next_attempt_ms: u64,
}

#[derive(Debug, Clone)]
pub struct PreparedChat {
    pub message_id: String,
    pub change: ChangeHash,
    pub durable: bool,
    pub replayed: bool,
}

impl ChatRecord {
    fn outcome(&self, replayed: bool) -> PreparedChat {
        PreparedChat {
            message_id: self.message_id.clone(),
            change: self.change,
            durable: self.durable,
            replayed,
        }
    }
}

impl DurableChatState {
    pub(crate) fn blocks(&self, doc_type: DocType, id: u128) -> bool {
        doc_type == DocType::Channel
            && self.active.iter().any(|token| {
                let r = &self.records[token];
                r.channel == id && !r.durable && !r.stopped
            })
    }

    pub(super) fn stop_obsolete(&mut self, context: Option<&[u8; 32]>) {
        self.active.retain(|token| {
            let record = self.records.get_mut(token).unwrap();
            if context != Some(&record.context) && !record.sealed.is_empty() {
                // This is a retained terminal disposition, not eviction. In particular an old
                // token can never become a fresh operation after the local sequence is freed.
                record.stopped = true;
                record.sealed.clear();
                return false;
            }
            true
        });
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SyncError> {
        let mut e = Encoder::new();
        e.put_u8(SNAPSHOT_VERSION);
        e.put_u32(self.records.len() as u32);
        for (token, r) in &self.records {
            e.put_bytes(token).map_err(|_| SyncError::Malformed)?;
            e.put_bytes(&r.binding).map_err(|_| SyncError::Malformed)?;
            e.put_bytes(&r.context).map_err(|_| SyncError::Malformed)?;
            e.put_u128(r.channel);
            e.put_str(&r.message_id).map_err(|_| SyncError::Malformed)?;
            e.put_bytes(&r.change.0).map_err(|_| SyncError::Malformed)?;
            e.put_u8(u8::from(r.durable) | (u8::from(r.stopped) << 1));
            e.put_bytes(&r.sealed).map_err(|_| SyncError::Malformed)?;
        }
        Ok(e.finish())
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let bad = || SyncError::Malformed;
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| bad())? != SNAPSHOT_VERSION {
            return Err(bad());
        }
        let count = d.get_u32().map_err(|_| bad())? as usize;
        if count > MAX_CHAT_RECEIPTS {
            return Err(bad());
        }
        let mut records = BTreeMap::new();
        let mut active = BTreeSet::new();
        let mut uncommitted_channels = BTreeSet::new();
        let mut pending = 0;
        for _ in 0..count {
            let token = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let binding = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let context = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let channel = d.get_u128().map_err(|_| bad())?;
            let message_id = d.get_str().map_err(|_| bad())?;
            if message_id.is_empty() || message_id.len() > 128 {
                return Err(bad());
            }
            let change = ChangeHash(
                d.get_bytes()
                    .map_err(|_| bad())?
                    .try_into()
                    .map_err(|_| bad())?,
            );
            let (durable, stopped) = match d.get_u8().map_err(|_| bad())? {
                0 => (false, false),
                1 => (true, false),
                2 => (false, true),
                3 => (true, true),
                _ => return Err(bad()),
            };
            let sealed = d.get_bytes().map_err(|_| bad())?;
            if sealed.len() > MAX_SEALED_CHAT
                || (!durable && !stopped && sealed.is_empty())
                || (stopped && !sealed.is_empty())
            {
                return Err(bad());
            }
            if !sealed.is_empty() {
                pending += 1;
                active.insert(token);
                let op = SealedOp::decode(sealed)?;
                if op.doc_type != DocType::Channel
                    || op.doc_id != channel
                    || pending > MAX_CHAT_PENDING
                {
                    return Err(bad());
                }
            }
            if !durable && !stopped && !uncommitted_channels.insert(channel) {
                return Err(bad());
            }
            if records
                .insert(
                    token,
                    ChatRecord {
                        binding,
                        context,
                        channel,
                        message_id: message_id.to_owned(),
                        change,
                        durable,
                        stopped,
                        sealed: sealed.to_vec(),
                        next_attempt_ms: 0,
                    },
                )
                .is_some()
            {
                return Err(bad());
            }
        }
        d.finish().map_err(|_| bad())?;
        Ok(Self { records, active })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Conservative authoring basis: any epoch change requires an explicit new user decision.
    /// It is not an owner receipt; ordinary messages require only current group membership.
    pub fn durable_send_context(&self) -> Result<[u8; 32], String> {
        if !self.group.contains_device(&self.device.device_id()) {
            return Err("CHAT_SEND_CONTEXT_CHANGED: this device is no longer a member".into());
        }
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/chat-send-context/v1");
        let group_id = self.group.group_id();
        h.update(&(group_id.len() as u64).to_be_bytes());
        h.update(&group_id);
        h.update(self.device.device_id().as_bytes());
        h.update(&self.group.epoch().to_be_bytes());
        Ok(*h.finalize().as_bytes())
    }

    /// Prepare once, outside the live/exportable document. `binding` is the canonical request
    /// digest including its original context. No transport method is called here.
    pub fn prepare_durable_chat<F>(
        &mut self,
        token: [u8; 16],
        binding: [u8; 32],
        expected_context: [u8; 32],
        channel: u128,
        message_id: String,
        edit: F,
    ) -> Result<PreparedChat, String>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), AutomergeError>,
    {
        let current_context = self.durable_send_context();
        self.durable_chat
            .stop_obsolete(current_context.as_ref().ok());
        if let Some(record) = self.durable_chat.records.get(&token) {
            if record.binding != binding
                || record.context != expected_context
                || record.channel != channel
            {
                return Err("CHAT_SEND_TOKEN_CONFLICT: token already binds another request".into());
            }
            // A saved receipt stays truthful after removal/epoch movement. Uncommitted work
            // must still pass current authority before it can cross the storage barrier.
            if !record.durable && (record.stopped || current_context? != expected_context) {
                return Err(
                    "CHAT_SEND_CONTEXT_CHANGED: prepared send belongs to an earlier epoch".into(),
                );
            }
            return Ok(record.outcome(true));
        }
        if current_context? != expected_context {
            return Err("CHAT_SEND_CONTEXT_CHANGED: intent belongs to an earlier epoch".into());
        }
        if self.durable_chat.records.len() >= MAX_CHAT_RECEIPTS
            || self.durable_chat.active.len() >= MAX_CHAT_PENDING
        {
            return Err("CHAT_SEND_CAPACITY: retained retry/publication capacity exhausted".into());
        }
        if self.durable_chat.blocks(DocType::Channel, channel) {
            return Err(
                "CHAT_SEND_DOCUMENT_PENDING: retry the outstanding send for this channel first"
                    .into(),
            );
        }
        if message_id.is_empty() || message_id.len() > 128 {
            return Err("invalid message id".into());
        }
        let original = self
            .docs
            .get_mut(&(DocType::Channel, channel))
            .ok_or_else(|| "document not open".to_string())?
            .snapshot()
            .map_err(|e| e.to_string())?;
        let mut candidate = EncryptedDoc::restore_for_actor(&original, &self.device.device_id())
            .map_err(|e| e.to_string())?;
        let (sealed, change) = candidate
            .edit_tracked(&self.device, &self.group, &mut self.rng, edit)
            .map_err(|e| e.to_string())?;
        let sealed = sealed.encode();
        if sealed.len() > MAX_SEALED_CHAT {
            return Err("CHAT_SEND_CAPACITY: signed operation exceeds preparation limit".into());
        }
        let record = ChatRecord {
            binding,
            context: expected_context,
            channel,
            message_id,
            change,
            durable: false,
            stopped: false,
            sealed,
            next_attempt_ms: 0,
        };
        let outcome = record.outcome(false);
        self.durable_chat.records.insert(token, record);
        self.durable_chat.active.insert(token);
        Ok(outcome)
    }

    /// Commit one prepared operation and its publication obligation in the SAME encrypted
    /// server snapshot. The caller must hold mounted store + exact native incarnation custody.
    /// A failed callback keeps the original live document and the exact preparation for retry.
    pub fn commit_durable_chat<F>(&mut self, token: [u8; 16], save: F) -> Result<(), String>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), String>,
    {
        let record = self
            .durable_chat
            .records
            .get(&token)
            .ok_or_else(|| "unknown chat preparation".to_string())?;
        if record.durable {
            return Ok(());
        }
        if record.stopped || self.durable_send_context()? != record.context {
            return Err(
                "CHAT_SEND_CONTEXT_CHANGED: prepared send belongs to an earlier epoch".into(),
            );
        }
        let key = (DocType::Channel, record.channel);
        let sealed = SealedOp::decode(&record.sealed).map_err(|e| e.to_string())?;
        let original_bytes = self
            .docs
            .get_mut(&key)
            .ok_or_else(|| "document not open".to_string())?
            .snapshot()
            .map_err(|e| e.to_string())?;
        let mut candidate =
            EncryptedDoc::restore_for_actor(&original_bytes, &self.device.device_id())
                .map_err(|e| e.to_string())?;
        candidate
            .ingest_tracked(&sealed, &self.group, &self.device)
            .map_err(|e| e.to_string())?;
        // Exclusive actor ownership and synchronous callback make this temporary swap invisible
        // to gossip, request serving and other commands. Never insert an await in this block.
        let original = self
            .docs
            .insert(key, candidate)
            .expect("open document checked above");
        self.durable_chat.records.get_mut(&token).unwrap().durable = true;
        let result = self
            .snapshot()
            .map_err(|e| e.to_string())
            .and_then(|snapshot| save(&snapshot, &mut self.rng));
        if result.is_err() {
            self.docs.insert(key, original);
            self.durable_chat.records.get_mut(&token).unwrap().durable = false;
        }
        result
    }

    /// Dedicated non-evicting replay queue. Cancellation retains the exact original operation;
    /// a crash can duplicate its submission, but cannot create another signed chat operation.
    pub(crate) async fn drain_durable_chat(&mut self) {
        let current_context = self.durable_send_context();
        self.durable_chat
            .stop_obsolete(current_context.as_ref().ok());
        let Ok(context) = current_context else {
            return;
        };
        let now = self.clock.monotonic_ms();
        let pending: Vec<_> = self
            .durable_chat
            .active
            .iter()
            .filter_map(|token| {
                let r = &self.durable_chat.records[token];
                (r.durable && r.context == context && r.next_attempt_ms <= now)
                    .then(|| (*token, r.channel, r.sealed.clone()))
            })
            .collect();
        for (token, channel, bytes) in pending {
            self.durable_chat
                .records
                .get_mut(&token)
                .unwrap()
                .next_attempt_ms = now.saturating_add(PUBLICATION_RETRY_MS);
            let Some(topic) = self.channel_topic_for(DocType::Channel, channel, self.routing_label)
            else {
                continue;
            };
            // Reserve cadence before the await: cancellation cannot turn a busy actor into a
            // submission storm. Only a driver acknowledgement retires the obligation.
            if matches!(
                self.transport.publish_once(topic, Bytes::from(bytes)).await,
                Ok(catcoms_rt::PublishSubmission::Submitted)
            ) {
                self.durable_chat
                    .records
                    .get_mut(&token)
                    .unwrap()
                    .sealed
                    .clear();
                self.durable_chat.active.remove(&token);
            }
        }
    }

    pub(crate) fn next_durable_chat_retry_delay(&self) -> Option<u64> {
        let now = self.clock.monotonic_ms();
        self.durable_chat
            .active
            .iter()
            .filter_map(|token| {
                let record = &self.durable_chat.records[token];
                record
                    .durable
                    .then(|| record.next_attempt_ms.saturating_sub(now))
            })
            .min()
    }
}
