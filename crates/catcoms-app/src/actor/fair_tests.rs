use super::*;
use bytes::Bytes;
use catcoms_mls::MlsDevice;
use catcoms_rt::{
    Hub, ManualClock, MemNetwork, PeerConnectionSnapshot, ProtocolId, SystemClock, Topic,
    TransportError, TransportEvent,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{Notify, Semaphore};

#[derive(Debug)]
struct Delay {
    hold: AtomicBool,
    maintenance: AtomicBool,
    started: Notify,
    release: Semaphore,
    requests: AtomicUsize,
    kinds: std::sync::Mutex<Vec<u8>>,
}
impl Delay {
    fn holds(&self, data: &[u8]) -> bool {
        self.hold.load(Ordering::SeqCst)
            && if self.maintenance.load(Ordering::SeqCst) {
                matches!(data.first(), Some(4 | 26 | 12))
            } else {
                is_channel_page(data)
            }
    }
    fn note_start(&self) {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
    }
}

#[derive(Debug)]
struct DelayedTransport {
    inner: MemNetwork,
    delay: Arc<Delay>,
}

fn is_channel_page(data: &[u8]) -> bool {
    if data.first() != Some(&19) {
        return false;
    }
    let mut frame = catcoms_wire::Decoder::new(&data[1..]);
    let Ok(inner) = frame.get_bytes() else {
        return false;
    };
    let mut request = catcoms_wire::Decoder::new(inner);
    matches!(request.get_u16(), Ok(tag) if tag == crate::DocType::Channel.tag())
        && matches!(request.get_u128(), Ok(1))
}

#[async_trait::async_trait]
impl MeshTransport for DelayedTransport {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        self.inner.connection_snapshot()
    }
    fn connection_lease(&self, peer: PeerId) -> Option<catcoms_rt::ConnectionLease> {
        self.inner.connection_lease(peer)
    }
    fn supports_connection_leases(&self) -> bool {
        self.inner.supports_connection_leases()
    }
    async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.subscribe(topic).await
    }
    async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
        self.inner.unsubscribe(topic).await
    }
    // All live gossip is dropped: the signed backlog must arrive through request/response.
    async fn publish(&self, _topic: Topic, _data: Bytes) -> Result<(), TransportError> {
        Ok(())
    }
    async fn request(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        if self.delay.maintenance.load(Ordering::SeqCst) {
            self.delay.kinds.lock().unwrap().push(data[0]);
        }
        let held = self.delay.holds(&data);
        if held && self.delay.maintenance.load(Ordering::SeqCst) {
            self.delay.note_start();
        }
        let response = self.inner.request(peer, protocol, data).await?;
        if held {
            if !self.delay.maintenance.load(Ordering::SeqCst) {
                self.delay.note_start();
            }
            self.delay.release.acquire().await.unwrap().forget();
        }
        Ok(response)
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
    async fn request_connected(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        if !self
            .connection_snapshot()
            .iter()
            .any(|row| row.peer == peer)
        {
            return Err(TransportError::Unreachable(peer));
        }
        self.request(peer, protocol, data).await
    }
    async fn request_connected_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        if !self
            .connection_snapshot()
            .iter()
            .any(|row| row.peer == peer)
        {
            return Err(TransportError::Unreachable(peer));
        }
        self.request_cancellable(peer, protocol, data, cancellation)
            .await
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
        mut cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        if self.delay.maintenance.load(Ordering::SeqCst) {
            self.delay.kinds.lock().unwrap().push(data[0]);
        }
        let held = self.delay.holds(&data);
        if held && self.delay.maintenance.load(Ordering::SeqCst) {
            self.delay.note_start();
        }
        let response = self
            .inner
            .request_cancellable(peer, protocol, data, cancellation.clone())
            .await?;
        if held {
            if !self.delay.maintenance.load(Ordering::SeqCst) {
                self.delay.note_start();
            }
            tokio::select! {
                permit = self.delay.release.acquire() => permit.unwrap().forget(),
                _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            }
        }
        Ok(response)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detached_member_maintenance_keeps_reciprocal_finalization_and_controls_responsive() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let delay = Arc::new(Delay {
        hold: AtomicBool::new(false),
        maintenance: AtomicBool::new(true),
        started: Notify::new(),
        release: Semaphore::new(0),
        requests: AtomicUsize::new(0),
        kinds: std::sync::Mutex::new(Vec::new()),
    });
    let a_peer = PeerId::from_u64(31);
    let b_peer = PeerId::from_u64(32);
    let net = |peer| DelayedTransport {
        inner: hub.join(peer),
        delay: delay.clone(),
    };
    let mut a = Server::found(
        net(a_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(31),
        Box::new(clock.clone()),
        "a",
    )
    .unwrap();
    a.subscribe_control().await.unwrap();
    let invite = a.mint_invite([31; 16], u64::MAX, vec![]).unwrap();
    let (joined, served) = tokio::join!(
        Server::join(
            net(b_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(32),
            Box::new(clock),
            "b",
            a_peer,
            &invite
        ),
        a.sync_once()
    );
    assert!(served.unwrap());
    let mut b = joined.unwrap();
    a.publish_self_record(Vec::new(), 1).unwrap();
    b.publish_self_record(Vec::new(), 1).unwrap();
    let (a, mut a_events, a_task) = spawn(a);
    let (b, mut b_events, b_task) = spawn(b);
    let a_drain = tokio::spawn(async move { while a_events.recv().await.is_some() {} });
    let b_drain = tokio::spawn(async move { while b_events.recv().await.is_some() {} });
    assert_eq!(bounded(a.member_count()).await, 2);
    assert_eq!(bounded(b.member_count()).await, 2);
    delay.hold.store(true, Ordering::SeqCst);
    let a_final = tokio::spawn({
        let a = a.clone();
        async move { a.finalize_member_connection(b_peer).await }
    });
    let b_final = tokio::spawn({
        let b = b.clone();
        async move { b.finalize_member_connection(a_peer).await }
    });
    bounded(async {
        while delay.requests.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    // The injected network clock is held still. Completion cannot rescue a blocked actor by
    // timing out, so these controls prove it has returned to the fair owner selector.
    for _ in 0..16 {
        assert_eq!(bounded(a.member_count()).await, 2);
        assert_eq!(bounded(b.member_count()).await, 2);
    }
    // Coalesce discovery while both explicit finalizations still own their replies. Releasing
    // them must start PEX and role refresh without another periodic discovery command.
    bounded(a.drive_discovery()).await.unwrap();
    bounded(b.drive_discovery()).await.unwrap();
    delay.release.add_permits(2);
    assert!(bounded(a_final).await.unwrap().unwrap());
    assert!(bounded(b_final).await.unwrap().unwrap());
    bounded(async {
        while delay.requests.load(Ordering::SeqCst) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(bounded(a.member_count()).await, 2);
    assert_eq!(bounded(b.member_count()).await, 2);
    delay.hold.store(false, Ordering::SeqCst);
    delay.release.add_permits(2);
    bounded(async {
        loop {
            if delay
                .kinds
                .lock()
                .unwrap()
                .iter()
                .filter(|kind| **kind == 12)
                .count()
                >= 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    bounded(a.shutdown()).await;
    bounded(b.shutdown()).await;
    bounded(a_task).await.unwrap();
    bounded(b_task).await.unwrap();
    a_drain.await.unwrap();
    b_drain.await.unwrap();
}

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::select! {
        result = future => result,
        _ = SystemClock.sleep(Duration::from_secs(10)) => panic!("actor work starved under command load"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_signed_history_completes_while_actor_commands_remain_saturated() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let delay = Arc::new(Delay {
        hold: AtomicBool::new(false),
        maintenance: AtomicBool::new(false),
        started: Notify::new(),
        release: Semaphore::new(0),
        requests: AtomicUsize::new(0),
        kinds: std::sync::Mutex::new(Vec::new()),
    });
    let a_peer = PeerId::from_u64(1);
    let b_peer = PeerId::from_u64(2);
    let net = |peer| DelayedTransport {
        inner: hub.join(peer),
        delay: Arc::clone(&delay),
    };
    let mut a = Server::found(
        net(a_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1),
        Box::new(clock.clone()),
        "a",
    )
    .unwrap();
    a.subscribe_control().await.unwrap();
    a.open_channel(1).await.unwrap();
    let invite = a.mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let (joined, served) = tokio::join!(
        Server::join(
            net(b_peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(clock.clone()),
            "b",
            a_peer,
            &invite
        ),
        a.sync_once()
    );
    assert!(served.unwrap());
    let mut b = joined.unwrap();
    b.open_channel(1).await.unwrap();
    // Learn the source through the production request-bound signature, never injected proof.
    let (proved, served) = tokio::join!(b.request_channel_catchup(a_peer, 1), a.sync_once());
    proved.unwrap();
    assert!(served.unwrap());
    a.send_message(1, "signed history during continuous reads")
        .await
        .unwrap();
    let expected = a.messages(1);
    assert_eq!(expected.len(), 1);
    assert!(b.messages(1).is_empty());
    assert!(b.sync.schedule_reconciliation() > 0);
    delay.hold.store(true, Ordering::SeqCst);
    let (a, mut a_events, a_task) = spawn(a);
    let (b, mut b_events, b_task) = spawn(b);
    let a_events_task = tokio::spawn(async move { while a_events.recv().await.is_some() {} });
    let b_events_task = tokio::spawn(async move { while b_events.recv().await.is_some() {} });
    let stop = Arc::new(AtomicBool::new(false));
    let commands = Arc::new(AtomicUsize::new(0));
    let mut readers = Vec::new();
    for _ in 0..8 {
        let actor = b.clone();
        let stop = Arc::clone(&stop);
        let commands = Arc::clone(&commands);
        readers.push(tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                actor.messages(1).await;
                commands.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    bounded(delay.started.notified()).await;
    let at_start = commands.load(Ordering::SeqCst);
    bounded(async {
        while commands.load(Ordering::SeqCst) < at_start + 128 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(
        delay.requests.load(Ordering::SeqCst),
        1,
        "commands retained one original response wait"
    );
    delay.hold.store(false, Ordering::SeqCst);
    delay.release.add_permits(1);
    let actual = bounded(async {
        loop {
            let messages = b.messages(1).await;
            if !messages.is_empty() {
                break messages;
            }
        }
    })
    .await;
    assert!(
        !stop.load(Ordering::SeqCst),
        "history arrived before command pressure stopped"
    );
    assert_eq!(
        actual, expected,
        "original IDs, authors and contents survive the delayed signed response"
    );
    stop.store(true, Ordering::SeqCst);
    for reader in readers {
        bounded(reader).await.unwrap();
    }
    a.shutdown().await;
    b.shutdown().await;
    bounded(a_task).await.unwrap();
    bounded(b_task).await.unwrap();
    a_events_task.await.unwrap();
    b_events_task.await.unwrap();
}
