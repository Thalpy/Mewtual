use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::store::epoch_budget::StorageScope;
use async_trait::async_trait;
use bytes::Bytes;
use catcoms_mls::MlsDevice;
use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp};
use catcoms_replication::{
    epoch_zero_id, DomainOp, InheritedCheckpoint, LogicalDocument, Receipt, SealedOp, SignedOp,
};
use catcoms_rt::{
    Hub, ManualClock, MemNetwork, PeerId, ProtocolId, PublishOnceError, RequestCancellation, Topic,
    TransportError, TransportEvent,
};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::sync::oneshot;

const SERVER: u64 = 88;
type Reply = Result<PublishSubmission, PublishOnceError>;

#[derive(Clone, Copy)]
enum Mode {
    Reply(Reply),
    Pending,
    Panic,
}

struct Wire {
    mode: Mutex<Mode>,
    sent: Mutex<Vec<(Topic, Bytes)>>,
    pending: Mutex<Vec<oneshot::Sender<Reply>>>,
}

#[derive(Clone)]
struct TestNet {
    inner: MemNetwork,
    wire: Arc<Wire>,
}

#[async_trait]
impl MeshTransport for TestNet {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    async fn subscribe(&self, t: Topic) -> Result<(), TransportError> {
        self.inner.subscribe(t).await
    }
    async fn unsubscribe(&self, t: Topic) -> Result<(), TransportError> {
        self.inner.unsubscribe(t).await
    }
    async fn publish(&self, _: Topic, _: Bytes) -> Result<(), TransportError> {
        panic!("no legacy outbox")
    }
    async fn publish_once(&self, t: Topic, b: Bytes) -> Reply {
        self.wire.sent.lock().unwrap().push((t, b));
        let mode = *self.wire.mode.lock().unwrap();
        match mode {
            Mode::Reply(reply) => reply,
            Mode::Pending => {
                let (tx, rx) = oneshot::channel();
                self.wire.pending.lock().unwrap().push(tx);
                rx.await.map_err(|_| PublishOnceError::Closed)?
            }
            Mode::Panic => panic!("injected publication unwind"),
        }
    }
    async fn request(
        &self,
        p: PeerId,
        proto: ProtocolId,
        b: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.inner.request(p, proto, b).await
    }
    async fn request_cancellable(
        &self,
        p: PeerId,
        proto: ProtocolId,
        b: Bytes,
        c: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.inner.request_cancellable(p, proto, b, c).await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
}

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(819)
}
fn poll_once<F: Future>(future: &mut Pin<Box<F>>) -> Poll<F::Output> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

struct Fixture {
    root: tempfile::TempDir,
    server: Server<TestNet, ChaCha20Rng>,
    net: TestNet,
    store: ServerStore,
    budget: EpochStorageBudget,
    intents: EpochIntentBudget,
    clock: ManualClock,
    key: PointerKey,
    document: LogicalDocument,
    doc_id: u128,
    domain: DomainOp,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let net = TestNet {
            inner: Hub::new().join(PeerId::from_u64(1)),
            wire: Arc::new(Wire {
                mode: Mutex::new(Mode::Reply(Ok(PublishSubmission::Submitted))),
                sent: Mutex::new(Vec::new()),
                pending: Mutex::new(Vec::new()),
            }),
        };
        let clock = ManualClock::new(0);
        let mut server = Server::found(
            net.clone(),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "replay",
        )
        .unwrap();
        let mut store = ServerStore::open(root.path(), b"replay-send", &mut rng()).unwrap();
        let key = PointerKey::new(DocType::StudioObject, b"private-sender-cat".to_vec()).unwrap();
        let document = registry_document(&server.group_id(), key.bucket()).unwrap();
        let doc_id = epoch_zero_id(DocType::DocRegistry, &document.logical_key);
        let domain = RegistryOp::Put {
            key: key.clone(),
            epoch: 2,
        }
        .domain_op(&server.group_id(), [1; 16])
        .unwrap();
        let (mut budget, mut intents) = inventory(&mut store, &server.group_id());
        // A real saved edit, with its canonical intent and source durable before any retry send.
        server
            .sync
            .with_registry_context(|g, d, _, r| {
                store.edit_registry_epoch(
                    SERVER,
                    g,
                    key.bucket(),
                    doc_id,
                    d,
                    domain.clone(),
                    r,
                    &mut budget,
                    &mut intents,
                )
            })
            .unwrap();
        Self {
            root,
            server,
            net,
            store,
            budget,
            intents,
            clock,
            key,
            document,
            doc_id,
            domain,
        }
    }
    fn begin(&mut self) -> ServerRegistryReplay {
        self.server
            .begin_registry_replay(
                &self.store,
                SERVER,
                self.key.bucket(),
                self.doc_id,
                &mut self.budget,
                &mut self.intents,
            )
            .unwrap()
    }
    async fn step(
        &mut self,
        cursor: &mut ServerRegistryReplay,
    ) -> Result<RegistryReplaySendStep, AppError> {
        self.server
            .send_registry_replay_step(&mut self.store, cursor, &mut self.budget, &mut self.intents)
            .await
    }
    fn pending(&self) -> usize {
        self.store
            .load_epoch_intents(SERVER, &self.document)
            .unwrap()
            .pending()
            .len()
    }
    fn state(&mut self) -> EpochRegistryState {
        self.server
            .sync
            .with_registry_context(|g, d, _, _| {
                self.store
                    .load_registry_epoch(SERVER, g, self.key.bucket(), d)
            })
            .unwrap()
            .unwrap()
    }
    fn packet(&self, index: usize) -> SealedOp {
        SealedOp::decode(&self.net.wire.sent.lock().unwrap()[index].1).unwrap()
    }
    fn signed(&mut self, index: usize) -> SignedOp {
        let packet = self.packet(index);
        self.server.sync.with_registry_context(|g, d, _, _| {
            packet
                .open(
                    &g.channel_secret(d, DocType::DocRegistry, self.doc_id)
                        .unwrap(),
                )
                .unwrap()
        })
    }
}

fn inventory(store: &mut ServerStore, group: &[u8]) -> (EpochStorageBudget, EpochIntentBudget) {
    let mut scan = store.scan_epoch_storage_with_registry().unwrap();
    while !scan.step().unwrap().complete {}
    let inv = scan.finish().unwrap();
    (
        EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, group).unwrap(),
            inv.records_for_server(SERVER, group).unwrap(),
        )
        .unwrap(),
        EpochIntentBudget::from_inventory(&inv).unwrap(),
    )
}

#[tokio::test]
async fn registry_send_submitted_advances_once_but_leaves_intent_and_source_durable() {
    let mut f = Fixture::new();
    let mut cursor = f.begin();
    assert_eq!(cursor.progress().selected, 1);
    assert_eq!(
        format!("{cursor:?}"),
        format!(
            "ServerRegistryReplay {{ progress: {:?}, .. }}",
            cursor.progress()
        )
    );
    let result = f.step(&mut cursor).await.unwrap();
    assert!(matches!(
        result,
        RegistryReplaySendStep::Attempt {
            result: Ok(PublishSubmission::Submitted),
            ..
        }
    ));
    assert!(!format!("{result:?}").contains("private-sender-cat"));
    assert_eq!(cursor.progress().submitted, 1);
    assert_eq!(f.pending(), 1, "local submission is not finality");
    assert_eq!(f.state().op_count(), 1);
    let signed = f.signed(0);
    assert_eq!(signed.domain_op.unwrap(), f.domain.encode().unwrap());
    assert!(matches!(
        f.step(&mut cursor).await.unwrap(),
        RegistryReplaySendStep::Complete
    ));
    assert_eq!(f.net.wire.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn registry_send_refusal_duplicate_and_retry_reseal_the_same_saved_operation() {
    let mut f = Fixture::new();
    let mut cursor = f.begin();
    let replies = [
        Ok(PublishSubmission::Duplicate),
        Err(PublishOnceError::NoPeers),
        Err(PublishOnceError::Busy),
        Err(PublishOnceError::Unsupported),
        Err(PublishOnceError::QueuesFull),
        Err(PublishOnceError::Closed),
        Err(PublishOnceError::TooLarge),
    ];
    for (index, reply) in replies.into_iter().enumerate() {
        *f.net.wire.mode.lock().unwrap() = Mode::Reply(reply);
        assert!(matches!(
            f.step(&mut cursor).await.unwrap(),
            RegistryReplaySendStep::Attempt { .. }
        ));
        assert_eq!(cursor.progress().visited, 0);
        assert_eq!(f.pending(), 1);
        assert!(matches!(
            f.step(&mut cursor).await.unwrap(),
            RegistryReplaySendStep::Wait { .. }
        ));
        if index > 0 {
            assert_ne!(
                f.packet(index).blob.ciphertext,
                f.packet(index - 1).blob.ciphertext
            );
            assert_eq!(
                f.signed(index),
                f.signed(0),
                "do not reauthor an exact retained retry"
            );
        }
        f.clock.advance_ms(100);
    }
    *f.net.wire.mode.lock().unwrap() = Mode::Reply(Ok(PublishSubmission::Submitted));
    f.step(&mut cursor).await.unwrap();
    assert_eq!(cursor.progress().submitted, 1);
    assert_eq!(f.state().op_count(), 1);
}

#[tokio::test]
async fn registry_send_pending_drop_ack_loss_and_unwind_never_skip_or_strand_ticket() {
    let mut f = Fixture::new();
    let mut cursor = f.begin();
    let wire = f.net.wire.clone();
    *wire.mode.lock().unwrap() = Mode::Pending;
    let mut future = Box::pin(f.step(&mut cursor));
    assert!(poll_once(&mut future).is_pending());
    assert_eq!(wire.sent.lock().unwrap().len(), 1);
    assert!(
        poll_once(&mut future).is_pending(),
        "no extra send while awaiting ack"
    );
    drop(future);
    assert!(wire.pending.lock().unwrap().pop().unwrap().is_closed());
    assert_eq!(cursor.progress().visited, 0);
    assert!(matches!(
        f.step(&mut cursor).await.unwrap(),
        RegistryReplaySendStep::Wait { .. }
    ));
    f.clock.advance_ms(100);
    let mut future = Box::pin(f.step(&mut cursor));
    assert!(poll_once(&mut future).is_pending());
    drop(wire.pending.lock().unwrap().pop().unwrap()); // Driver/ack lost after possible admission.
    assert!(matches!(
        future.await.unwrap(),
        RegistryReplaySendStep::Attempt {
            result: Err(SyncError::Publication(PublishOnceError::Closed)),
            ..
        }
    ));
    assert_eq!(cursor.progress().visited, 0);
    f.clock.advance_ms(100);
    *wire.mode.lock().unwrap() = Mode::Panic;
    let mut future = Box::pin(f.step(&mut cursor));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll_once(&mut future))).is_err()
    );
    drop(future);
    assert_eq!(cursor.progress().visited, 0);
    f.clock.advance_ms(100);
    *wire.mode.lock().unwrap() = Mode::Reply(Ok(PublishSubmission::Submitted));
    f.step(&mut cursor).await.unwrap();
    assert_eq!(cursor.progress().submitted, 1);
    for i in 1..4 {
        assert_eq!(f.signed(i), f.signed(0));
    }
    assert_eq!(f.pending(), 1);
}

#[tokio::test]
async fn registry_send_rejects_replaced_server_and_reopened_mount_before_publication() {
    let mut f = Fixture::new();
    let mut cursor = f.begin();
    let snapshot = f.server.snapshot().unwrap();
    let mut replacement = Server::restore(
        &snapshot,
        f.net.clone(),
        rng(),
        Box::new(f.clock.clone()),
        "replacement",
    )
    .unwrap();
    assert_eq!(replacement.group_id(), f.server.group_id());
    assert_eq!(replacement.device_id(), f.server.device_id());
    assert!(replacement
        .send_registry_replay_step(&mut f.store, &mut cursor, &mut f.budget, &mut f.intents)
        .await
        .unwrap_err()
        .to_string()
        .contains("replaced server"));
    assert_eq!(cursor.progress().visited, 0);
    assert!(f.net.wire.sent.lock().unwrap().is_empty());
    drop(f.store);
    f.store = ServerStore::open(f.root.path(), b"replay-send", &mut rng()).unwrap();
    (f.budget, f.intents) = inventory(&mut f.store, &f.server.group_id());
    assert!(f
        .step(&mut cursor)
        .await
        .unwrap_err()
        .to_string()
        .contains("another mount"));
    assert!(f.net.wire.sent.lock().unwrap().is_empty());
    let mut fresh = f.begin();
    f.step(&mut fresh).await.unwrap();
    assert_eq!(fresh.progress().submitted, 1);
}

#[tokio::test]
async fn registry_send_storage_failure_and_closing_gate_never_publish() {
    let mut f = Fixture::new();
    let mut cursor = f.begin();
    f.budget.invalidate();
    assert!(f.step(&mut cursor).await.is_err());
    assert!(matches!(
        f.step(&mut cursor).await.unwrap(),
        RegistryReplaySendStep::Paused
    ));
    assert!(f.net.wire.sent.lock().unwrap().is_empty());
    (f.budget, f.intents) = inventory(&mut f.store, &f.server.group_id());
    cursor.retry_failed().unwrap();
    f.clock.advance_ms(100);
    let checkpoint = f.state().projection().unwrap().checkpoint([9; 32]).unwrap();
    f.server.sync.with_registry_context(|g, d, _, r| {
        let receipt = Receipt::sign(
            f.document.clone(),
            0,
            [9; 32],
            checkpoint.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        f.store
            .seal_registry_epoch(SERVER, g, f.key.bucket(), d, receipt, 0, r, &mut f.budget)
            .unwrap();
    });
    assert!(f.step(&mut cursor).await.is_err());
    assert!(f.net.wire.sent.lock().unwrap().is_empty());
    assert_eq!(f.pending(), 1);
    assert_eq!(f.state().op_count(), 1);
}

#[tokio::test]
async fn registry_send_conservative_hold_visits_but_does_not_publish_or_retire() {
    let mut f = Fixture::new();
    let old = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 1,
    }
    .domain_op(&f.server.group_id(), [2; 16])
    .unwrap();
    f.server
        .sync
        .with_registry_context(|g, d, _, r| {
            f.store.prepare_epoch_intent(
                SERVER,
                &f.document,
                old,
                d,
                g,
                r,
                &mut f.budget,
                &mut f.intents,
            )
        })
        .unwrap();
    let mut cursor = f.begin();
    let mut held = 0;
    for _ in 0..2 {
        if matches!(
            f.step(&mut cursor).await.unwrap(),
            RegistryReplaySendStep::Held {
                reason: RegistryReplayHold::SupersededPointer,
                ..
            }
        ) {
            held += 1;
        }
        f.clock.advance_ms(100);
    }
    assert_eq!(held, 1);
    assert_eq!(cursor.progress().held, 1);
    assert_eq!(cursor.progress().submitted, 1);
    assert_eq!(f.net.wire.sent.lock().unwrap().len(), 1);
    assert_eq!(f.pending(), 2);
    assert!(matches!(
        f.step(&mut cursor).await.unwrap(),
        RegistryReplaySendStep::Complete
    ));
}
