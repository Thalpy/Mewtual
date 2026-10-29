//! Private local chat preparation, exact-token receipts and synchronous storage transactions.
//! Candidates remain absent from every live/history export until the encrypted snapshot barrier.
//! A replacement whose parent sync failed is hidden uncertainty, never proof of a rollback.
use super::*;
use catcoms_replication::doc::SavedLocalChatOp;

pub const MAX_CHAT_RECEIPTS: usize = 65_536;
pub const MAX_CHAT_PENDING: usize = 32;
const MAX_SEALED_CHAT: usize = 256 * 1024;
const MAX_NOOP_HEADS: usize = 256;
const PUBLICATION_RETRY_MS: u64 = 1_000;

/// Exact saved-request fence: retry token, canonical request binding, original context and
/// channel. Keeping all four fields together prevents a token-only match from stopping a
/// different request. This in-process alias does not change the persisted fence format.
pub type SavedChatFenceIdentity = ([u8; 16], [u8; 32], [u8; 32], u128);

/// Preserve the actual storage replacement boundary across crates. In particular, a parent
/// sync failure follows a successful rename and cannot be reported as a failed replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatSaveError {
    NotReplaced(String),
    ReplacedNotDurable(String),
}
impl std::fmt::Display for ChatSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReplaced(e) | Self::ReplacedNotDurable(e) => f.write_str(e),
        }
    }
}
impl From<String> for ChatSaveError {
    fn from(e: String) -> Self {
        Self::NotReplaced(e)
    }
}
impl From<&str> for ChatSaveError {
    fn from(e: &str) -> Self {
        Self::NotReplaced(e.into())
    }
}

/// Closed request kind. Send's original binding/domain/wire bytes remain unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAction {
    Send,
    Edit,
    Delete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatDisposition {
    Committed,
    StoppedBeforeCommit,
    StoppedAfterCommit,
}

#[derive(Clone, Default)]
pub(crate) struct DurableChatState {
    records: BTreeMap<[u8; 16], ChatRecord>,
    active: BTreeSet<[u8; 16]>,
    // Entire possible terminals, including metadata-only Stop/no-op. Never cleared by epochs.
    replacement_uncertain: BTreeMap<[u8; 16], ChatRecord>,
    v2: bool,
    // Only the synchronous reconciliation transaction may snapshot a hidden candidate.
    reconciling: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::{transaction::Transactable, ReadDoc, ROOT};
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
        assert_eq!(error.to_string(), "injected disk failure");
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
            DurableChatState::decode(&encoded, &n.device)
                .unwrap()
                .records
                .len(),
            MAX_CHAT_PENDING
        );
        let mut unknown = encoded;
        unknown[0] = 255;
        assert!(DurableChatState::decode(&unknown, &n.device).is_err());
    }

    fn receive_foreign(n: &mut Node, key: &str) {
        let foreign = MlsDevice::generate().unwrap();
        let mut graph = n.doc(DocType::Channel, 5).unwrap().doc().clone();
        graph.set_actor(automerge::ActorId::from(
            foreign.device_id().as_bytes().to_vec(),
        ));
        graph
            .put(ROOT, key, "received while storage was uncertain")
            .unwrap();
        graph.commit();
        let delta = graph.get_last_local_change().unwrap().raw_bytes().to_vec();
        let op = catcoms_replication::SignedOp::sign(&foreign, DocType::Channel, 5, delta).unwrap();
        let sealed = SealedOp::seal(&op, &n.group, &n.device, &mut n.rng).unwrap();
        n.docs
            .get_mut(&(DocType::Channel, 5))
            .unwrap()
            .ingest_tracked(&sealed, &n.group, &n.device)
            .unwrap();
    }

    #[tokio::test]
    async fn durable_chat_uncertain_stop_merges_latest_foreign_receive_across_epoch() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        let context = n.durable_send_context().unwrap();
        let prepared = prepare(&mut n, 1, 5);
        let original = prepared.change;
        n.commit_durable_chat([1; 16], |_, _| {
            Err(ChatSaveError::ReplacedNotDurable("parent sync".into()))
        })
        .unwrap_err();
        receive_foreign(&mut n, "foreign");
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 1);
        assert!(n.snapshot().is_err());
        let peer = MlsDevice::generate().unwrap();
        n.with_observed_mls_transition(|n| {
            n.group.add_member(&n.device, peer.key_package().unwrap())
        })
        .unwrap();
        n.drain_durable_chat().await;
        assert!(n.durable_chat_needs_repair());
        let stopped = n
            .stop_durable_chat(
                [1; 16],
                [1; 32],
                context,
                5,
                "m1".into(),
                ChatAction::Send,
                None,
                |bytes, _| {
                    let disk = restore(bytes);
                    assert_eq!(disk.doc(DocType::Channel, 5).unwrap().op_count(), 2);
                    assert!(disk
                        .doc(DocType::Channel, 5)
                        .unwrap()
                        .doc()
                        .get(ROOT, "foreign")
                        .unwrap()
                        .is_some());
                    Ok(())
                },
            )
            .unwrap();
        assert!(stopped.durable);
        assert_eq!(stopped.change, original);
        assert_eq!(stopped.disposition, ChatDisposition::StoppedAfterCommit);
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 2);
        assert!(n.snapshot().is_ok());
        assert!(n.transport.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn durable_chat_possible_stop_is_monotone_through_failure_epoch_and_late_positive() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        let context = n.durable_send_context().unwrap();
        n.stop_durable_chat(
            [7; 16],
            [7; 32],
            context,
            5,
            "m7".into(),
            ChatAction::Send,
            None,
            |_, _| Err(ChatSaveError::ReplacedNotDurable("rename happened".into())),
        )
        .unwrap_err();
        n.stop_durable_chat(
            [7; 16],
            [7; 32],
            context,
            5,
            "m7".into(),
            ChatAction::Send,
            None,
            |_, _| Err(ChatSaveError::NotReplaced("still unavailable".into())),
        )
        .unwrap_err();
        assert!(n.snapshot().is_err());
        let peer = MlsDevice::generate().unwrap();
        n.with_observed_mls_transition(|n| {
            n.group.add_member(&n.device, peer.key_package().unwrap())
        })
        .unwrap();
        let possible = n
            .prepare_durable_chat([7; 16], [7; 32], context, 5, "m7".into(), |_| {
                panic!("a possible Stop cannot author")
            })
            .unwrap();
        assert!(!possible.durable);
        assert_eq!(possible.disposition, ChatDisposition::StoppedBeforeCommit);
        n.commit_durable_chat([7; 16], |_, _| Ok(())).unwrap();
        let saved = n
            .durable_chat_lookup([7; 16], [7; 32], context, 5)
            .unwrap()
            .unwrap();
        assert!(saved.durable);
        assert_eq!(saved.disposition, ChatDisposition::StoppedBeforeCommit);
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 0);
        let encoded = n.durable_chat.encode().unwrap();
        assert_eq!(encoded[0], 2, "old v1 reader rejects before records");
        let disk = restore(&n.snapshot().unwrap());
        assert!(disk.durable_chat.records[&[7; 16]].explicit_stop);
    }

    #[tokio::test]
    async fn durable_chat_save_unwind_restores_live_graph_and_latches_whole_possible_stop() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        let context = n.durable_send_context().unwrap();
        prepare(&mut n, 1, 5);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            n.commit_durable_chat([1; 16], |_, _| panic!("save callback unwound"))
        }));
        assert!(panic.is_err());
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 0);
        assert!(n.snapshot().is_err());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            n.stop_durable_chat(
                [1; 16],
                [1; 32],
                context,
                5,
                "m1".into(),
                ChatAction::Send,
                None,
                |_, _| panic!("stop save callback unwound"),
            )
        }));
        assert!(panic.is_err());
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 0);
        let possible = n
            .durable_chat_lookup([1; 16], [1; 32], context, 5)
            .unwrap()
            .unwrap();
        assert_eq!(possible.disposition, ChatDisposition::StoppedAfterCommit);
        n.repair_durable_chat(|_, _| Ok(())).unwrap();
        assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 1);
        assert_eq!(
            n.durable_chat.records[&[1; 16]].outcome(true).disposition,
            ChatDisposition::StoppedAfterCommit
        );
    }
    #[tokio::test]
    async fn durable_chat_noop_refuses_real_frontier_over_retained_head_cap() {
        let mut n = node();
        n.open_channel(DocType::Channel, 5).await.unwrap();
        for index in 0..=MAX_NOOP_HEADS {
            let mut graph = AutoCommit::new();
            let mut actor = vec![0; 16];
            actor[..8].copy_from_slice(&(index as u64).to_be_bytes());
            graph.set_actor(automerge::ActorId::from(actor));
            graph.put(ROOT, format!("head-{index}"), true).unwrap();
            graph.commit();
            let op = catcoms_replication::SignedOp::sign(
                &n.device,
                DocType::Channel,
                5,
                graph.get_last_local_change().unwrap().raw_bytes().to_vec(),
            )
            .unwrap();
            let sealed = SealedOp::seal(&op, &n.group, &n.device, &mut n.rng).unwrap();
            n.docs
                .get_mut(&(DocType::Channel, 5))
                .unwrap()
                .ingest_tracked(&sealed, &n.group, &n.device)
                .unwrap();
        }
        assert_eq!(
            n.doc(DocType::Channel, 5)
                .unwrap()
                .doc()
                .clone()
                .get_heads()
                .len(),
            MAX_NOOP_HEADS + 1
        );
        let context = n.durable_send_context().unwrap();
        let error = n
            .prepare_durable_chat_action(
                [1; 16],
                [1; 32],
                context,
                5,
                "sample".into(),
                ChatAction::Edit,
                Some([1; 32]),
                true,
                |_| panic!("no-op never edits"),
            )
            .unwrap_err();
        assert!(error.contains("no-op frontier exceeds"));
        assert!(n.durable_chat.records.is_empty());
    }
}

#[derive(Clone)]
enum ChatEffect {
    Signed {
        change: ChangeHash,
        original: Option<SavedLocalChatOp>,
    },
    NoOp {
        heads: Vec<ChangeHash>,
    },
    NeverAuthored,
}
#[derive(Clone)]
struct ChatRecord {
    binding: [u8; 32],
    context: [u8; 32],
    channel: u128,
    message_id: String,
    action: ChatAction,
    target: Option<[u8; 32]>,
    effect: ChatEffect,
    durable: bool,
    stopped: bool,
    explicit_stop: bool,
    sealed: Vec<u8>,
    next_attempt_ms: u64,
}
#[derive(Debug, Clone)]
pub struct PreparedChat {
    pub message_id: String,
    pub change: Option<ChangeHash>,
    pub durable: bool,
    pub replayed: bool,
    pub disposition: ChatDisposition,
}
impl ChatRecord {
    fn outcome(&self, replayed: bool) -> PreparedChat {
        PreparedChat {
            message_id: self.message_id.clone(),
            change: match self.effect {
                ChatEffect::Signed { change, .. } => Some(change),
                _ => None,
            },
            durable: self.durable || self.explicit_stop,
            replayed,
            disposition: if self.explicit_stop {
                if self.durable {
                    ChatDisposition::StoppedAfterCommit
                } else {
                    ChatDisposition::StoppedBeforeCommit
                }
            } else {
                ChatDisposition::Committed
            },
        }
    }
}
impl DurableChatState {
    pub(crate) fn snapshot_allowed(&self) -> bool {
        self.reconciling || self.replacement_uncertain.is_empty()
    }
    pub(crate) fn blocks(&self, doc_type: DocType, id: u128) -> bool {
        doc_type == DocType::Channel
            && (self
                .records
                .values()
                .any(|r| r.channel == id && !r.durable && !r.stopped)
                || self.replacement_uncertain.values().any(|r| r.channel == id))
    }
    fn rebuild_active(&mut self) {
        self.active = self
            .records
            .iter()
            .filter_map(|(token, r)| {
                ((!r.durable && !r.stopped) || !r.sealed.is_empty()).then_some(*token)
            })
            .collect();
        self.active
            .extend(self.replacement_uncertain.keys().copied());
    }
    pub(super) fn stop_obsolete(&mut self, context: Option<&[u8; 32]>) {
        // Ordinary owner turns visit only bounded active work, not all non-evicting receipts.
        // Uncertainty owns its original payload and possible terminal until a covering repair.
        for token in self.active.clone() {
            if self.replacement_uncertain.contains_key(&token) {
                continue;
            }
            let Some(record) = self.records.get_mut(&token) else {
                continue;
            };
            if context != Some(&record.context) && !record.stopped {
                record.stopped = true;
                record.sealed.clear();
                if let ChatEffect::Signed { original, .. } = &mut record.effect {
                    *original = None;
                }
            }
        }
        let records = &self.records;
        let uncertain = &self.replacement_uncertain;
        self.active.retain(|token| {
            uncertain.contains_key(token)
                || records
                    .get(token)
                    .is_some_and(|r| (!r.durable && !r.stopped) || !r.sealed.is_empty())
        });
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, SyncError> {
        let mut e = Encoder::new();
        e.put_u8(if self.v2 { 2 } else { 1 });
        e.put_u32(self.records.len() as u32);
        for (token, r) in &self.records {
            e.put_bytes(token).map_err(|_| SyncError::Malformed)?;
            e.put_bytes(&r.binding).map_err(|_| SyncError::Malformed)?;
            e.put_bytes(&r.context).map_err(|_| SyncError::Malformed)?;
            e.put_u128(r.channel);
            e.put_str(&r.message_id).map_err(|_| SyncError::Malformed)?;
            if self.v2 {
                e.put_u8(match r.action {
                    ChatAction::Send => 0,
                    ChatAction::Edit => 1,
                    ChatAction::Delete => 2,
                });
                if let Some(target) = r.target {
                    e.put_bytes(&target).map_err(|_| SyncError::Malformed)?;
                }
                match &r.effect {
                    ChatEffect::Signed { change, original } => {
                        e.put_u8(0);
                        e.put_bytes(&change.0).map_err(|_| SyncError::Malformed)?;
                        e.put_bytes(
                            &original
                                .as_ref()
                                .map_or_else(Vec::new, SavedLocalChatOp::encode),
                        )
                        .map_err(|_| SyncError::Malformed)?;
                    }
                    ChatEffect::NoOp { heads } => {
                        e.put_u8(1);
                        e.put_u32(heads.len() as u32);
                        for h in heads {
                            e.put_bytes(&h.0).map_err(|_| SyncError::Malformed)?;
                        }
                    }
                    ChatEffect::NeverAuthored => {
                        e.put_u8(2);
                    }
                }
            } else {
                let ChatEffect::Signed { change, .. } = r.effect else {
                    return Err(SyncError::Malformed);
                };
                e.put_bytes(&change.0).map_err(|_| SyncError::Malformed)?;
            }
            e.put_u8(
                u8::from(r.durable)
                    | (u8::from(r.stopped) << 1)
                    | (u8::from(self.v2 && r.explicit_stop) << 2),
            );
            e.put_bytes(&r.sealed).map_err(|_| SyncError::Malformed)?;
        }
        Ok(e.finish())
    }
    pub(crate) fn decode(bytes: &[u8], device: &MlsDevice) -> Result<Self, SyncError> {
        let bad = || SyncError::Malformed;
        let mut d = Decoder::new(bytes);
        let version = d.get_u8().map_err(|_| bad())?;
        if !matches!(version, 1 | 2) {
            return Err(bad());
        }
        let count = d.get_u32().map_err(|_| bad())? as usize;
        if count > MAX_CHAT_RECEIPTS {
            return Err(bad());
        }
        let mut this = Self {
            v2: version == 2,
            ..Self::default()
        };
        let mut channels = BTreeSet::new();
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
            let message_id = d.get_str().map_err(|_| bad())?.to_string();
            if message_id.is_empty() || message_id.len() > 128 {
                return Err(bad());
            }
            let action = if version == 1 {
                ChatAction::Send
            } else {
                match d.get_u8().map_err(|_| bad())? {
                    0 => ChatAction::Send,
                    1 => ChatAction::Edit,
                    2 => ChatAction::Delete,
                    _ => return Err(bad()),
                }
            };
            let target = if action == ChatAction::Send {
                None
            } else {
                Some(
                    d.get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?,
                )
            };
            let effect_tag = if version == 1 {
                0
            } else {
                d.get_u8().map_err(|_| bad())?
            };
            let effect = match effect_tag {
                0 => {
                    let change = ChangeHash(
                        d.get_bytes()
                            .map_err(|_| bad())?
                            .try_into()
                            .map_err(|_| bad())?,
                    );
                    let original = if version == 1 {
                        None
                    } else {
                        let raw = d.get_bytes().map_err(|_| bad())?;
                        if raw.len() > MAX_SEALED_CHAT {
                            return Err(bad());
                        }
                        if raw.is_empty() {
                            None
                        } else {
                            Some(SavedLocalChatOp::decode(raw, device, channel, change)?)
                        }
                    };
                    ChatEffect::Signed { change, original }
                }
                1 => {
                    let n = d.get_u32().map_err(|_| bad())? as usize;
                    if n > MAX_NOOP_HEADS || action != ChatAction::Edit {
                        return Err(bad());
                    }
                    let mut heads = Vec::with_capacity(n);
                    for _ in 0..n {
                        heads.push(ChangeHash(
                            d.get_bytes()
                                .map_err(|_| bad())?
                                .try_into()
                                .map_err(|_| bad())?,
                        ));
                    }
                    if heads.windows(2).any(|p| p[0] >= p[1]) {
                        return Err(bad());
                    }
                    ChatEffect::NoOp { heads }
                }
                2 if version == 2 => ChatEffect::NeverAuthored,
                _ => return Err(bad()),
            };
            let flags = d.get_u8().map_err(|_| bad())?;
            if flags > if version == 1 { 3 } else { 7 } {
                return Err(bad());
            }
            let durable = flags & 1 != 0;
            let stopped = flags & 2 != 0;
            let explicit_stop = flags & 4 != 0;
            let sealed = d.get_bytes().map_err(|_| bad())?;
            let raw_len = match &effect {
                ChatEffect::Signed {
                    original: Some(op), ..
                } => op.encode().len(),
                _ => 0,
            };
            if sealed.len().saturating_add(raw_len) > MAX_SEALED_CHAT
                || (stopped && !sealed.is_empty())
                || (explicit_stop && !stopped)
                || (matches!(effect, ChatEffect::NeverAuthored) && (!explicit_stop || durable))
                || (!matches!(effect, ChatEffect::Signed { .. }) && !sealed.is_empty())
                || (matches!(effect, ChatEffect::Signed { .. })
                    && !durable
                    && !stopped
                    && sealed.is_empty())
            {
                return Err(bad());
            }
            if !sealed.is_empty() {
                let op = SealedOp::decode(sealed)?;
                if op.doc_type != DocType::Channel || op.doc_id != channel {
                    return Err(bad());
                }
            }
            if (!durable && !stopped) || !sealed.is_empty() {
                pending += 1;
                if pending > MAX_CHAT_PENDING {
                    return Err(bad());
                }
            }
            if !durable && !stopped && !channels.insert(channel) {
                return Err(bad());
            }
            let record = ChatRecord {
                binding,
                context,
                channel,
                message_id,
                action,
                target,
                effect,
                durable,
                stopped,
                explicit_stop,
                sealed: sealed.to_vec(),
                next_attempt_ms: 0,
            };
            if this.records.insert(token, record).is_some() {
                return Err(bad());
            }
        }
        d.finish().map_err(|_| bad())?;
        this.rebuild_active();
        Ok(this)
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn durable_send_context(&self) -> Result<[u8; 32], String> {
        if !self.group.contains_device(&self.device.device_id()) {
            return Err("CHAT_SEND_CONTEXT_CHANGED: this device is no longer a member".into());
        }
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/chat-send-context/v1");
        let id = self.group.group_id();
        h.update(&(id.len() as u64).to_be_bytes());
        h.update(&id);
        h.update(self.device.device_id().as_bytes());
        h.update(&self.group.epoch().to_be_bytes());
        Ok(*h.finalize().as_bytes())
    }
    /// Exact binding lookup precedes permission/target reads, so saved deletion receipts remain
    /// replayable after the row disappears. An uncertain result is never claimed durable.
    pub fn durable_chat_lookup(
        &self,
        token: [u8; 16],
        binding: [u8; 32],
        context: [u8; 32],
        channel: u128,
    ) -> Result<Option<PreparedChat>, String> {
        let uncertain = self.durable_chat.replacement_uncertain.get(&token);
        let Some(r) = uncertain.or_else(|| self.durable_chat.records.get(&token)) else {
            return Ok(None);
        };
        if r.binding != binding || r.context != context || r.channel != channel {
            return Err("CHAT_SEND_TOKEN_CONFLICT: token already binds another request".into());
        }
        let mut result = r.outcome(true);
        if uncertain.is_some() {
            result.durable = false;
        }
        Ok(Some(result))
    }
    pub fn durable_chat_token_needs_repair(&self, token: [u8; 16]) -> bool {
        self.durable_chat.replacement_uncertain.contains_key(&token)
    }
    /// Only repaired signed effects can change projections; metadata-only terminals are omitted.
    pub fn durable_chat_repair_channels(&self) -> Vec<u128> {
        self.durable_chat
            .replacement_uncertain
            .values()
            .filter_map(|r| {
                (r.durable && matches!(r.effect, ChatEffect::Signed { .. })).then_some(r.channel)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn durable_chat_needs_repair(&self) -> bool {
        !self.durable_chat.replacement_uncertain.is_empty()
    }
    pub fn prepare_durable_chat<F>(
        &mut self,
        token: [u8; 16],
        binding: [u8; 32],
        context: [u8; 32],
        channel: u128,
        message_id: String,
        edit: F,
    ) -> Result<PreparedChat, String>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), AutomergeError>,
    {
        self.prepare_durable_chat_action(
            token,
            binding,
            context,
            channel,
            message_id,
            ChatAction::Send,
            None,
            false,
            edit,
        )
    }
    /// Prepare an action after product authorization. Every uncommitted retry must pass that
    /// authorization again; this layer separately retains context and exact target identity.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_durable_chat_action<F>(
        &mut self,
        token: [u8; 16],
        binding: [u8; 32],
        context: [u8; 32],
        channel: u128,
        message_id: String,
        action: ChatAction,
        target: Option<[u8; 32]>,
        noop: bool,
        edit: F,
    ) -> Result<PreparedChat, String>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), AutomergeError>,
    {
        let current = self.durable_send_context();
        self.durable_chat.stop_obsolete(current.as_ref().ok());
        if let Some(found) = self.durable_chat_lookup(token, binding, context, channel)? {
            if found.durable || self.durable_chat.replacement_uncertain.contains_key(&token) {
                return Ok(found);
            }
            let r = &self.durable_chat.records[&token];
            if r.stopped || current? != context {
                return Err(
                    "CHAT_SEND_CONTEXT_CHANGED: preparation belongs to an earlier epoch".into(),
                );
            }
            return Ok(found);
        }
        if current? != context {
            return Err("CHAT_SEND_CONTEXT_CHANGED: intent belongs to an earlier epoch".into());
        }
        if self.durable_chat.records.len() >= MAX_CHAT_RECEIPTS
            || self.durable_chat.active.len() >= MAX_CHAT_PENDING
        {
            return Err("CHAT_SEND_CAPACITY: retained retry/publication capacity exhausted".into());
        }
        if self.durable_chat.blocks(DocType::Channel, channel) {
            return Err(
                "CHAT_SEND_DOCUMENT_PENDING: retry the outstanding action for this channel first"
                    .into(),
            );
        }
        if message_id.is_empty()
            || message_id.len() > 128
            || (action == ChatAction::Send) != target.is_none()
            || (noop && action != ChatAction::Edit)
        {
            return Err("invalid chat target".into());
        }
        let source = self
            .docs
            .get_mut(&(DocType::Channel, channel))
            .ok_or_else(|| "document not open".to_string())?;
        let original = source.snapshot().map_err(|e| e.to_string())?;
        let mut candidate = EncryptedDoc::restore_for_actor(&original, &self.device.device_id())
            .map_err(|e| e.to_string())?;
        let (effect, sealed) = if noop {
            // Bound retained receipt metadata. Automerge materializes its existing frontier
            // before this check (like the candidate snapshot itself, transient work scales with
            // the existing graph). No oversized frontier enters a prepared receipt.
            // A sampled no-op never turns into a later edit.
            let heads = candidate.doc().clone().get_heads();
            if heads.len() > MAX_NOOP_HEADS {
                return Err("CHAT_SEND_CAPACITY: no-op frontier exceeds limit".into());
            }
            (ChatEffect::NoOp { heads }, Vec::new())
        } else {
            let (sealed, change) = candidate
                .edit_tracked(&self.device, &self.group, &mut self.rng, edit)
                .map_err(|e| e.to_string())?;
            let op =
                EncryptedDoc::capture_local_chat_op(&sealed, &self.group, &self.device, change)
                    .map_err(|e| e.to_string())?;
            let bytes = sealed.encode();
            if bytes.len().saturating_add(op.encode().len()) > MAX_SEALED_CHAT {
                return Err(
                    "CHAT_SEND_CAPACITY: signed operation exceeds preparation limit".into(),
                );
            }
            (
                ChatEffect::Signed {
                    change,
                    original: Some(op),
                },
                bytes,
            )
        };
        self.durable_chat.v2 |= action != ChatAction::Send || noop;
        let r = ChatRecord {
            binding,
            context,
            channel,
            message_id,
            action,
            target,
            effect,
            durable: false,
            stopped: false,
            explicit_stop: false,
            sealed,
            next_attempt_ms: 0,
        };
        let outcome = r.outcome(false);
        self.durable_chat.records.insert(token, r);
        self.durable_chat.rebuild_active();
        Ok(outcome)
    }
    /// Save an exact-token Stop even if its original positive request never reached this Actor.
    /// This negative transaction performs no fresh authorization or operation authoring.
    #[allow(clippy::too_many_arguments)]
    pub fn stop_durable_chat<F>(
        &mut self,
        token: [u8; 16],
        binding: [u8; 32],
        context: [u8; 32],
        channel: u128,
        message_id: String,
        action: ChatAction,
        target: Option<[u8; 32]>,
        save: F,
    ) -> Result<PreparedChat, ChatSaveError>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), ChatSaveError>,
    {
        if let Some(saved) = self
            .durable_chat_lookup(token, binding, context, channel)
            .map_err(ChatSaveError::NotReplaced)?
        {
            if saved.durable && saved.disposition != ChatDisposition::Committed {
                return Ok(saved);
            }
        }
        if message_id.is_empty()
            || message_id.len() > 128
            || (action == ChatAction::Send) != target.is_none()
        {
            return Err(ChatSaveError::NotReplaced("invalid chat target".into()));
        }
        let mut record = self
            .durable_chat
            .replacement_uncertain
            .get(&token)
            .or_else(|| self.durable_chat.records.get(&token))
            .cloned()
            .unwrap_or(ChatRecord {
                binding,
                context,
                channel,
                message_id,
                action,
                target,
                effect: ChatEffect::NeverAuthored,
                durable: false,
                stopped: true,
                explicit_stop: true,
                sealed: Vec::new(),
                next_attempt_ms: 0,
            });
        if !self.durable_chat.records.contains_key(&token)
            && !self.durable_chat.replacement_uncertain.contains_key(&token)
            && self.durable_chat.records.len() >= MAX_CHAT_RECEIPTS
        {
            return Err(ChatSaveError::NotReplaced(
                "CHAT_SEND_CAPACITY: retained receipt capacity exhausted".into(),
            ));
        }
        record.stopped = true;
        record.explicit_stop = true;
        record.sealed.clear();
        if !record.durable {
            record.effect = ChatEffect::NeverAuthored;
        }
        self.chat_storage_transaction(Some((token, record)), true, save)?;
        Ok(self.durable_chat.records[&token].outcome(true))
    }
    pub fn commit_durable_chat<F>(&mut self, token: [u8; 16], save: F) -> Result<(), ChatSaveError>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), ChatSaveError>,
    {
        if self.durable_chat.replacement_uncertain.contains_key(&token) {
            return self.repair_durable_chat(save);
        }
        let record = self
            .durable_chat
            .records
            .get(&token)
            .ok_or_else(|| ChatSaveError::NotReplaced("unknown chat preparation".into()))?;
        if record.durable || record.explicit_stop {
            return Ok(());
        }
        if record.stopped
            || self
                .durable_send_context()
                .map_err(ChatSaveError::NotReplaced)?
                != record.context
        {
            return Err(ChatSaveError::NotReplaced(
                "CHAT_SEND_CONTEXT_CHANGED: preparation belongs to an earlier epoch".into(),
            ));
        }
        let mut committed = record.clone();
        committed.durable = true;
        self.chat_storage_transaction(Some((token, committed)), false, save)
    }
    /// Reconcile original possibly installed effects with the LATEST receive graph. This is
    /// negative history repair, independent of lock, role, epoch and renderer availability.
    pub fn repair_durable_chat<F>(&mut self, save: F) -> Result<(), ChatSaveError>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), ChatSaveError>,
    {
        if !self.durable_chat_needs_repair() {
            return Ok(());
        }
        self.chat_storage_transaction(None, false, save)
    }
    /// Restored autonomous publication is subject to retired exact-token fences too. This
    /// negative path only stops receipts already in this snapshot; it admits no new token.
    pub fn fence_saved_durable_chat<F>(
        &mut self,
        identities: &[SavedChatFenceIdentity],
        save: F,
    ) -> Result<(), ChatSaveError>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), ChatSaveError>,
    {
        let mut stopped = BTreeMap::new();
        for &(token, binding, context, channel) in identities {
            let Some(saved) = self
                .durable_chat_lookup(token, binding, context, channel)
                .map_err(ChatSaveError::NotReplaced)?
            else {
                continue;
            };
            if saved.durable && saved.disposition != ChatDisposition::Committed {
                continue;
            }
            let mut record = self.durable_chat.records[&token].clone();
            record.stopped = true;
            record.explicit_stop = true;
            record.sealed.clear();
            if !record.durable {
                record.effect = ChatEffect::NeverAuthored;
            }
            stopped.insert(token, record);
        }
        if stopped.is_empty() {
            return Ok(());
        }
        self.chat_storage_transaction_records(stopped, true, save)
    }
    fn chat_storage_transaction<F>(
        &mut self,
        selected: Option<([u8; 16], ChatRecord)>,
        v2: bool,
        save: F,
    ) -> Result<(), ChatSaveError>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), ChatSaveError>,
    {
        self.chat_storage_transaction_records(selected.into_iter().collect(), v2, save)
    }
    fn chat_storage_transaction_records<F>(
        &mut self,
        selected: BTreeMap<[u8; 16], ChatRecord>,
        explicit_v2: bool,
        save: F,
    ) -> Result<(), ChatSaveError>
    where
        F: FnOnce(&[u8], &mut R) -> Result<(), ChatSaveError>,
    {
        let mut updates = self.durable_chat.replacement_uncertain.clone();
        updates.extend(selected);
        let mut previous = self.durable_chat.clone();
        let mut originals = BTreeMap::new();
        // Build every repaired document from its latest live state. Never replace it with an
        // earlier prepared graph: that would drop foreign operations received during disk churn.
        let mut candidates = BTreeMap::new();
        for r in updates.values_mut() {
            if !r.durable {
                continue;
            }
            let ChatEffect::Signed { change, original } = &mut r.effect else {
                continue;
            };
            let key = (DocType::Channel, r.channel);
            if let std::collections::btree_map::Entry::Vacant(entry) = candidates.entry(key) {
                let raw = self
                    .docs
                    .get_mut(&key)
                    .ok_or_else(|| ChatSaveError::NotReplaced("document not open".into()))?
                    .snapshot()
                    .map_err(|e| ChatSaveError::NotReplaced(e.to_string()))?;
                entry.insert(
                    EncryptedDoc::restore_for_actor(&raw, &self.device.device_id())
                        .map_err(|e| ChatSaveError::NotReplaced(e.to_string()))?,
                );
            }
            if original.is_none() && r.sealed.is_empty() {
                // A known saved operation may already have retired publication and its private
                // payload. Stop retains its real hash/history, never demands a stale MLS key.
                if automerge::ReadDoc::get_change_by_hash(candidates[&key].doc(), change).is_some()
                {
                    continue;
                }
                return Err(ChatSaveError::NotReplaced(
                    "saved chat history is missing its exact operation".into(),
                ));
            }
            if original.is_none() {
                let sealed = SealedOp::decode(&r.sealed)
                    .map_err(|e| ChatSaveError::NotReplaced(e.to_string()))?;
                *original = Some(
                    EncryptedDoc::capture_local_chat_op(
                        &sealed,
                        &self.group,
                        &self.device,
                        *change,
                    )
                    .map_err(|e| ChatSaveError::NotReplaced(e.to_string()))?,
                );
            }
            candidates
                .get_mut(&key)
                .unwrap()
                .repair_local_chat_op(original.as_ref().unwrap())
                .map_err(|e| ChatSaveError::NotReplaced(e.to_string()))?;
        }
        // Cache legacy originals BEFORE the storage callback can install their bytes. A later
        // epoch change cannot then prevent repair of a possible replacement.
        for (t, r) in &updates {
            if let Some(old) = self.durable_chat.records.get_mut(t) {
                if let (
                    ChatEffect::Signed { original: old, .. },
                    ChatEffect::Signed { original: new, .. },
                ) = (&mut old.effect, &r.effect)
                {
                    *old = new.clone();
                }
            }
        }
        for (token, record) in &self.durable_chat.records {
            if let Some(old) = previous.records.get_mut(token) {
                old.effect = record.effect.clone();
            }
        }
        for (key, candidate) in candidates {
            originals.insert(key, self.docs.insert(key, candidate).unwrap());
        }
        for (t, r) in &updates {
            let mut installed = r.clone();
            if installed.stopped {
                if let ChatEffect::Signed { original, .. } = &mut installed.effect {
                    *original = None;
                }
            }
            self.durable_chat.records.insert(*t, installed);
        }
        self.durable_chat.v2 |= explicit_v2;
        self.durable_chat.reconciling = true;
        self.durable_chat.rebuild_active();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let snapshot = self
                .snapshot()
                .map_err(|e| ChatSaveError::NotReplaced(e.to_string()))?;
            save(&snapshot, &mut self.rng)
        }));
        match result {
            Ok(Ok(())) => {
                self.durable_chat.replacement_uncertain.clear();
                self.durable_chat.reconciling = false;
                self.durable_chat.rebuild_active();
                Ok(())
            }
            failed => {
                for (key, original) in originals {
                    self.docs.insert(key, original);
                }
                self.durable_chat = previous;
                let uncertain = matches!(
                    &failed,
                    Err(_) | Ok(Err(ChatSaveError::ReplacedNotDurable(_)))
                );
                if uncertain {
                    self.durable_chat.v2 |= explicit_v2;
                    for (token, record) in updates {
                        self.durable_chat
                            .replacement_uncertain
                            .insert(token, record);
                    }
                    self.durable_chat.rebuild_active();
                }
                match failed {
                    Ok(Err(e)) => Err(e),
                    Err(panic) => std::panic::resume_unwind(panic),
                    Ok(Ok(())) => unreachable!(),
                }
            }
        }
    }
    pub(crate) async fn drain_durable_chat(&mut self) {
        let current = self.durable_send_context();
        self.durable_chat.stop_obsolete(current.as_ref().ok());
        let Ok(context) = current else {
            return;
        };
        let now = self.clock.monotonic_ms();
        let pending: Vec<_> = self
            .durable_chat
            .active
            .iter()
            .filter_map(|t| {
                let r = self.durable_chat.records.get(t)?;
                (r.durable
                    && !r.stopped
                    && r.context == context
                    && !r.sealed.is_empty()
                    && r.next_attempt_ms <= now
                    && !self.durable_chat.replacement_uncertain.contains_key(t))
                .then(|| (*t, r.channel, r.sealed.clone()))
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
            if matches!(
                self.transport.publish_once(topic, Bytes::from(bytes)).await,
                Ok(catcoms_rt::PublishSubmission::Submitted)
            ) {
                let r = self.durable_chat.records.get_mut(&token).unwrap();
                r.sealed.clear();
                if let ChatEffect::Signed { original, .. } = &mut r.effect {
                    *original = None;
                }
                self.durable_chat.active.remove(&token);
            }
        }
    }
    pub(crate) fn next_durable_chat_retry_delay(&self) -> Option<u64> {
        let now = self.clock.monotonic_ms();
        self.durable_chat
            .active
            .iter()
            .filter_map(|t| {
                let r = self.durable_chat.records.get(t)?;
                (r.durable
                    && !r.stopped
                    && !r.sealed.is_empty()
                    && !self.durable_chat.replacement_uncertain.contains_key(t))
                .then(|| r.next_attempt_ms.saturating_sub(now))
            })
            .min()
    }
}
