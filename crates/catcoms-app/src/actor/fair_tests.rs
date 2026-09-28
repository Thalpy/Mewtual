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
    started: Notify,
    release: Semaphore,
    requests: AtomicUsize,
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
        let held = self.delay.hold.load(Ordering::SeqCst) && is_channel_page(&data);
        let response = self.inner.request(peer, protocol, data).await?;
        if held {
            self.delay.requests.fetch_add(1, Ordering::SeqCst);
            self.delay.started.notify_one();
            self.delay.release.acquire().await.unwrap().forget();
        }
        Ok(response)
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
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
        started: Notify::new(),
        release: Semaphore::new(0),
        requests: AtomicUsize::new(0),
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
