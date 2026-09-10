use super::*;
use crate::studio::StudioRequest;
use async_trait::async_trait;
use bytes::Bytes;
use catcoms_mls::MlsDevice;
use catcoms_replication::studio::{
    FlipnoteHeader, FlipnoteOp, IndexOp, StudioExpiry, StudioKind, StudioProjection,
};
use catcoms_rt::{
    Hub, ManualClock, MemNetwork, PeerId, ProtocolId, PublishOnceError, RequestCancellation, Topic,
    TransportError, TransportEvent,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Waker};
mod actor_save;
mod controls;
mod discovery;
mod pages;
mod receiver;
mod reconnect;
mod registry_runtime;
mod replay;
mod unopened;

const SERVER: u64 = 83;
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(903)
}
fn channel() -> [u8; 16] {
    crate::channel_id("general").to_be_bytes()
}
fn target() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: channel(),
        object: [7; 16],
    }
}
fn open(path: &std::path::Path) -> ServerStore {
    ServerStore::open(path, b"studio-exchange", &mut rng()).unwrap()
}
fn domain(target: StudioTarget, body: Vec<u8>, nonce: u8) -> DomainOp {
    let logical = target.document(b"only-type-and-key-used").unwrap();
    DomainOp {
        doc_type: logical.doc_type,
        logical_key: logical.logical_key,
        body,
        nonce: [nonce; 16],
    }
}
fn title(nonce: u8, text: &str) -> DomainOp {
    domain(
        target(),
        FlipnoteOp::SetHeader(FlipnoteHeader::Title(text.into()))
            .encode()
            .unwrap(),
        nonce,
    )
}

// Real MemNetwork for membership/routing; the sole injection pauses exactly at publish_once,
// AFTER saved-only validation and both durability barriers. No sleeps or detached retry task.
#[derive(Clone)]
struct Net {
    inner: MemNetwork,
    pause: Arc<AtomicBool>,
    attempts: Arc<AtomicUsize>,
    // Optional deterministic test-control permits, used only by the automatic Save regressions.
    release: Arc<tokio::sync::Semaphore>,
    controlled: Arc<AtomicBool>,
    started: Arc<tokio::sync::Notify>,
}
impl Net {
    fn new(inner: MemNetwork) -> Self {
        Self {
            inner,
            pause: Arc::new(AtomicBool::new(false)),
            attempts: Arc::new(AtomicUsize::new(0)),
            release: Arc::new(tokio::sync::Semaphore::new(0)),
            controlled: Arc::new(AtomicBool::new(false)),
            started: Arc::new(tokio::sync::Notify::new()),
        }
    }
}
#[async_trait]
impl MeshTransport for Net {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<catcoms_rt::PeerConnectionSnapshot> {
        self.inner.connection_snapshot()
    }
    async fn request_connected_cancellable(
        &self,
        p: PeerId,
        proto: ProtocolId,
        b: Bytes,
        c: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.inner
            .request_connected_cancellable(p, proto, b, c)
            .await
    }
    async fn subscribe(&self, t: Topic) -> Result<(), TransportError> {
        self.inner.subscribe(t).await
    }
    async fn unsubscribe(&self, t: Topic) -> Result<(), TransportError> {
        self.inner.unsubscribe(t).await
    }
    async fn publish(&self, t: Topic, b: Bytes) -> Result<(), TransportError> {
        self.inner.publish(t, b).await
    }
    async fn publish_once(
        &self,
        t: Topic,
        b: Bytes,
    ) -> Result<PublishSubmission, PublishOnceError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        if self.controlled.load(Ordering::SeqCst) {
            self.release.acquire().await.unwrap().forget();
        }
        if self.pause.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.inner.publish_once(t, b).await
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
type Node = Server<Net, ChaCha20Rng>;
fn budget(node: &mut Node, store: &mut ServerStore) -> EpochStudioBudget {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inv = scan.finish().unwrap();
    node.sync
        .with_registry_context(|g, _, _, _| store.studio_storage_budget(SERVER, g, &inv))
        .unwrap()
}
struct Pair {
    hub: Arc<Hub>,
    _a_root: tempfile::TempDir,
    b_root: tempfile::TempDir,
    alice: Node,
    bob: Node,
    a_store: ServerStore,
    b_store: ServerStore,
    wire: Net,
    watch: ServerStudioWatch,
    clock: ManualClock,
}
impl Pair {
    async fn new() -> Self {
        let hub = Hub::new();
        let wire = Net::new(hub.join(PeerId::from_u64(1)));
        let bob_wire = Net::new(hub.join(PeerId::from_u64(2)));
        let clock = ManualClock::new(1000);
        let mut alice = Server::found(
            wire.clone(),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        let invite = alice.mint_invite([1; 16], u64::MAX, vec![]).unwrap();
        let (bob, tick) = tokio::join!(
            Server::join(
                bob_wire,
                MlsDevice::generate().unwrap(),
                rng(),
                Box::new(clock.clone()),
                "bob",
                alice.local_peer(),
                &invite
            ),
            alice.sync_once()
        );
        tick.unwrap();
        let mut bob = bob.unwrap();
        let a_root = tempfile::tempdir().unwrap();
        let b_root = tempfile::tempdir().unwrap();
        let a_store = open(a_root.path());
        let b_store = open(b_root.path());
        let watch = bob.watch_studio_epoch(&b_store, SERVER, target()).unwrap();
        bob.flush_studio_subscriptions().await.unwrap();
        Self {
            hub,
            _a_root: a_root,
            b_root,
            alice,
            bob,
            a_store,
            b_store,
            wire,
            watch,
            clock,
        }
    }
    fn save(&mut self, op: &DomainOp) -> u128 {
        let doc = target().document(&self.alice.group_id()).unwrap();
        let id = epoch_zero_id(doc.doc_type, &doc.logical_key);
        self.alice
            .studio_transaction(
                &mut self.a_store,
                SERVER,
                StudioRequest::Apply {
                    target: target(),
                    epoch_id: id,
                    nonce: op.nonce,
                    body: op.body.clone(),
                },
            )
            .unwrap();
        id
    }
    async fn send(&mut self, op: DomainOp) -> Result<PublishSubmission, AppError> {
        let mut b = budget(&mut self.alice, &mut self.a_store);
        let doc = target().document(&self.alice.group_id()).unwrap();
        let id = epoch_zero_id(doc.doc_type, &doc.logical_key);
        self.alice
            .send_saved_studio_once(&mut self.a_store, SERVER, target(), id, op, &mut b)
            .await
    }
    fn receive(&mut self) -> Result<Option<StudioReceived>, AppError> {
        let mut b = budget(&mut self.bob, &mut self.b_store);
        self.bob
            .receive_studio_step(&mut self.b_store, &self.watch, &mut b)
    }
    fn state(&mut self) -> Option<EpochStudioState> {
        self.bob
            .sync
            .with_registry_context(|g, d, _, _| {
                self.b_store.load_studio_epoch(SERVER, g, target(), d)
            })
            .unwrap()
    }
    fn pending(&self) -> usize {
        self.a_store
            .load_epoch_intents(SERVER, &target().document(&self.alice.group_id()).unwrap())
            .unwrap()
            .pending()
            .len()
    }
}

#[tokio::test]
async fn studio_exchange_two_members_save_real_frame_receive_duplicate_and_reopen() {
    let mut p = Pair::new().await;
    let header = title(1, "private moon");
    p.save(&header);
    assert_eq!(
        p.send(header.clone()).await.unwrap(),
        PublishSubmission::Submitted
    );
    p.bob.sync_once().await.unwrap();
    assert!(p.state().is_none(), "authenticated queue is not saved");
    assert_eq!(p.receive().unwrap().unwrap().admission, Admission::Accepted);
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    // Actual normal publication, not a synthetic frame hash.
    p.alice.set_blob_store(
        p.a_store
            .blob_store(&hex::encode(p.alice.group_id()))
            .unwrap(),
    );
    let published = p.alice.publish_pix(&pix).unwrap();
    let cid = crate::Cid::from_hex(&published.cid).unwrap();
    let frame = domain(
        target(),
        FlipnoteOp::InsertFrame {
            frame: [4; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pix.len() as u64,
        }
        .encode()
        .unwrap(),
        2,
    );
    p.save(&frame);
    p.send(frame.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let accepted = p.receive().unwrap().unwrap();
    assert_eq!(accepted.admission, Admission::Accepted);
    let StudioProjection::Flipnote(view) = accepted.state.projection().unwrap() else {
        panic!()
    };
    assert_eq!(view.timeline, vec![[4; 16]]);
    assert!(
        !p.b_store
            .blob_store(&hex::encode(p.bob.group_id()))
            .unwrap()
            .has(&cid),
        "no auto-fetch or invented byte possession"
    );
    p.send(frame).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert_eq!(
        p.receive().unwrap().unwrap().admission,
        Admission::Duplicate
    );
    assert_eq!(p.pending(), 2, "send never retires intents");
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    assert_eq!(p.state().unwrap().op_count(), 2);
    assert!(p.receive().is_err(), "old mounted watch revoked");
    p.bob.unwatch_studio_epoch(&p.watch).unwrap();
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target())
        .unwrap();
    p.send(header).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert_eq!(
        p.receive().unwrap().unwrap().admission,
        Admission::Duplicate
    );
    assert!(!format!("{:?}", p.watch).contains("private moon"));
}

#[tokio::test]
async fn studio_exchange_unsaved_send_and_changed_nonce_never_create_intents_or_source() {
    let mut p = Pair::new().await;
    let op = title(1, "saved");
    assert!(p.send(op.clone()).await.is_err());
    assert_eq!(p.pending(), 0);
    assert_eq!(p.wire.attempts.load(Ordering::SeqCst), 0);
    p.save(&op);
    assert!(p.send(title(2, "saved")).await.is_err());
    assert!(p.send(title(1, "different body")).await.is_err());
    assert_eq!(p.pending(), 1);
    assert_eq!(p.wire.attempts.load(Ordering::SeqCst), 0);
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert_eq!(p.receive().unwrap().unwrap().admission, Admission::Accepted);
}

#[tokio::test]
async fn studio_exchange_missing_dependency_is_not_saved_and_exact_retry_recovers() {
    let mut p = Pair::new().await;
    let first = title(1, "first");
    let second = title(2, "second");
    p.save(&first);
    p.save(&second);
    p.send(second.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert!(p.receive().is_err());
    assert!(p.state().is_none());
    p.send(first).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap().unwrap();
    p.send(second).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert_eq!(p.receive().unwrap().unwrap().state.op_count(), 2);
}

#[tokio::test]
async fn studio_exchange_cancellation_after_durable_retry_keeps_saved_intent() {
    let mut p = Pair::new().await;
    let op = title(1, "retry");
    let id = p.save(&op);
    let mut b = budget(&mut p.alice, &mut p.a_store);
    p.wire.pause.store(true, Ordering::SeqCst);
    let mut send = Box::pin(p.alice.send_saved_studio_once(
        &mut p.a_store,
        SERVER,
        target(),
        id,
        op.clone(),
        &mut b,
    ));
    assert!(send
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    assert_eq!(p.wire.attempts.load(Ordering::SeqCst), 1);
    drop(send);
    assert_eq!(p.pending(), 1);
    assert!(p.state().is_none());
    p.wire.pause.store(false, Ordering::SeqCst);
    p.send(op.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap().unwrap();
    p.bob.unwatch_studio_epoch(&p.watch).unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    assert!(p.send(op).await.is_err());
    assert_eq!(p.pending(), 1);
}

#[tokio::test]
async fn studio_exchange_channel_binding_and_rewatch_reject_stale_queue() {
    let mut p = Pair::new().await;
    let op = title(1, "moon");
    p.save(&op);
    let bad = StudioTarget::Flipnote {
        channel: [0; 16],
        object: [7; 16],
    };
    assert!(p.bob.watch_studio_epoch(&p.b_store, SERVER, bad).is_err());
    p.send(op.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let old = std::mem::replace(
        &mut p.watch,
        p.bob
            .watch_studio_epoch(&p.b_store, SERVER, target())
            .unwrap(),
    );
    let mut b = budget(&mut p.bob, &mut p.b_store);
    assert!(p
        .bob
        .receive_studio_step(&mut p.b_store, &old, &mut b)
        .is_err());
    assert!(p.receive().unwrap().is_none());
    assert!(p.bob.unwatch_studio_epoch(&old).is_err());
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap().unwrap();
    // The same physical object under another EXISTING channel must fail typed root binding.
    // Create the channel on Bob locally; transport authentication alone cannot prove root channel.
    p.bob.open_channel_index().await.unwrap();
    let alternate = p
        .bob
        .create_channel("alternate")
        .await
        .unwrap()
        .id
        .to_be_bytes();
    let bad = StudioTarget::Flipnote {
        channel: alternate,
        object: [7; 16],
    };
    assert!(p.bob.watch_studio_epoch(&p.b_store, SERVER, bad).is_err());
}

#[tokio::test]
async fn studio_exchange_index_uses_same_transport_and_typed_store_gate() {
    let mut p = Pair::new().await;
    let target = StudioTarget::Index { channel: channel() };
    let creator = p
        .alice
        .sync
        .with_registry_context(|_, d, _, _| d.device_id());
    let op = domain(
        target,
        IndexOp::PutObject {
            object: [7; 16],
            kind: StudioKind::Flipnote,
            title: "moon".into(),
            created_by: creator,
            ts: 1000,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        3,
    );
    let logical = target.document(&p.alice.group_id()).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Apply {
                target,
                epoch_id: id,
                nonce: op.nonce,
                body: op.body.clone(),
            },
        )
        .unwrap();
    let watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    let mut a = budget(&mut p.alice, &mut p.a_store);
    p.alice
        .send_saved_studio_once(&mut p.a_store, SERVER, target, id, op, &mut a)
        .await
        .unwrap();
    p.bob.sync_once().await.unwrap();
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let got = p
        .bob
        .receive_studio_step(&mut p.b_store, &watch, &mut b)
        .unwrap()
        .unwrap();
    let StudioProjection::Index(view) = got.state.projection().unwrap() else {
        panic!()
    };
    assert_eq!(view.objects.len(), 1);
    assert_eq!(got.admission, Admission::Accepted);
    // Incoming data never creates an own intent on the receiver.
    assert_eq!(
        p.b_store
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        0
    );
}

#[tokio::test]
async fn studio_exchange_receipt_after_queueing_quarantines_and_prevents_resending_closed_work() {
    use catcoms_replication::{EpochPhase, InheritedCheckpoint, Receipt};
    let mut p = Pair::new().await;
    let first = title(1, "before seal");
    let second = title(2, "queued at seal");
    p.save(&first);
    p.send(first.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let prior = p.receive().unwrap().unwrap().state;
    p.save(&second);
    p.send(second).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let logical = target().document(&p.alice.group_id()).unwrap();
    // This test isolates receipt admission, not automatic eligible-close production (gate 4).
    // The current owner's signed selection closes the exact source while a packet is queued.
    let receipt = p.alice.sync.with_registry_context(|_, device, _, _| {
        Receipt::sign(
            logical,
            0,
            [8; 32],
            prior
                .projection()
                .unwrap()
                .checkpoint([8; 32])
                .unwrap()
                .change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            device,
        )
        .unwrap()
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .sync
        .with_registry_context(|g, d, _, r| {
            p.b_store
                .seal_studio_epoch(SERVER, g, target(), d, receipt.clone(), 0, r, &mut b)
        })
        .unwrap();
    let late = p.receive().unwrap().unwrap();
    assert_eq!(late.admission, Admission::Quarantined);
    assert_eq!(late.state.phase(), EpochPhase::Closing);
    assert_eq!(late.state.op_count(), 1);
    assert_eq!(late.state.quarantined_len(), 1);
    let mut a = budget(&mut p.alice, &mut p.a_store);
    p.alice
        .sync
        .with_registry_context(|g, d, _, r| {
            p.a_store
                .seal_studio_epoch(SERVER, g, target(), d, receipt, 0, r, &mut a)
        })
        .unwrap();
    let attempts = p.wire.attempts.load(Ordering::SeqCst);
    assert!(p.send(first).await.is_err());
    assert_eq!(p.wire.attempts.load(Ordering::SeqCst), attempts);
    assert_eq!(p.pending(), 2);
}

#[tokio::test]
async fn studio_exchange_nonowner_edits_received_source_and_owner_reopens_reply() {
    let mut p = Pair::new().await;
    let initial = title(1, "alice's first version");
    let id = p.save(&initial);
    p.send(initial).await.unwrap();
    p.bob.sync_once().await.unwrap();
    p.receive().unwrap().unwrap();
    let reply = title(2, "bob's next version");
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: id,
                nonce: reply.nonce,
                body: reply.body.clone(),
            },
        )
        .unwrap();
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    p.alice.flush_studio_subscriptions().await.unwrap();
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .send_saved_studio_once(&mut p.b_store, SERVER, target(), id, reply, &mut b)
        .await
        .unwrap();
    p.alice.sync_once().await.unwrap();
    let mut a = budget(&mut p.alice, &mut p.a_store);
    let got = p
        .alice
        .receive_studio_step(&mut p.a_store, &watch, &mut a)
        .unwrap()
        .unwrap();
    assert_eq!(got.admission, Admission::Accepted);
    assert_eq!(got.state.op_count(), 2);
    let StudioProjection::Flipnote(view) = got.state.projection().unwrap() else {
        panic!()
    };
    let title = view.title.as_ref().unwrap();
    assert_eq!(title.selected.value, "bob's next version");
    let bob = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    assert_eq!(title.selected.source.author, bob);
    let projection = got.state.projection().unwrap();
    drop(p.a_store);
    p.a_store = open(p._a_root.path());
    let reopened = p
        .alice
        .sync
        .with_registry_context(|g, d, _, _| p.a_store.load_studio_epoch(SERVER, g, target(), d))
        .unwrap()
        .unwrap();
    assert_eq!(reopened.projection().unwrap(), projection);
    assert_eq!(
        p.pending(),
        1,
        "receiving another author's edit creates no own intent"
    );
}

#[tokio::test]
async fn studio_exchange_absent_object_wrong_channel_rejects_at_durable_admission() {
    let mut p = Pair::new().await;
    let op = title(1, "authored in general");
    p.save(&op);
    p.bob.open_channel_index().await.unwrap();
    let alternate = p
        .bob
        .create_channel("alternate")
        .await
        .unwrap()
        .id
        .to_be_bytes();
    let wrong = StudioTarget::Flipnote {
        channel: alternate,
        object: [7; 16],
    };
    p.watch = p.bob.watch_studio_epoch(&p.b_store, SERVER, wrong).unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert!(
        p.receive().is_err(),
        "signed envelope is insufficient: root has the wrong channel"
    );
    assert!(p.state().is_none());
    assert!(
        p.receive().unwrap().is_none(),
        "bad packet was consumed, not retried implicitly"
    );
    let logical = wrong.document(&p.bob.group_id()).unwrap();
    assert_eq!(
        p.b_store
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        0
    );
}
