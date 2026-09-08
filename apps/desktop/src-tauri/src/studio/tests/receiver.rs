use super::*;
use catcoms_rt::{Clock, MemNetwork};
use std::pin::Pin;
use std::time::Duration;

// Exposes the actual background pacing boundary without wall sleeps or scheduling guesses.
#[derive(Debug, Clone)]
struct PaceClock {
    inner: ManualClock,
    sleeping: Arc<tokio::sync::Notify>,
}
impl PaceClock {
    fn new() -> Self {
        Self {
            inner: ManualClock::new(1000),
            sleeping: Default::default(),
        }
    }
}
impl Clock for PaceClock {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        self.inner.monotonic_ms()
    }
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let wait = self.inner.sleep(duration);
        self.sleeping.notify_one();
        wait
    }
}
async fn while_driving<T, F: Future<Output = ()>>(
    driver: &mut Pin<Box<F>>,
    work: impl Future<Output = T>,
) -> T {
    tokio::select! { biased;
        _ = driver => panic!("receiver stopped unexpectedly"),
        value = work => value,
    }
}
struct Peer {
    _root: tempfile::TempDir,
    state: Arc<AppState>,
    actor: ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
    updates: mpsc::UnboundedReceiver<Option<[u8; 16]>>,
}
impl Peer {
    async fn new(mut server: Server<MemNetwork, ChaCha20Rng>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
        *state.session_resumable.lock().await = true;
        attach_blob_store(&state, &mut server).await;
        let group = server.group_id();
        let device = server.device_id();
        let (actor, mut events, task) = spawn(server);
        install(&state, actor.clone(), group, device, 1).await;
        let (tx, updates) = mpsc::unbounded_channel();
        let drain = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event.event {
                    AppEvent::StudioUpdated { object, .. } => {
                        let _ = tx.send(object);
                    }
                    AppEvent::StudioReceivePaused => {
                        panic!("small healthy test source must not pause")
                    }
                    _ => {}
                }
            }
        });
        Self {
            _root: root,
            state,
            actor,
            task,
            drain,
            updates,
        }
    }
    fn driver(&self, clock: PaceClock) -> tokio::task::JoinHandle<()> {
        let state = self.state.clone();
        let actor = self.actor.clone();
        tokio::spawn(async move { drive_receiver(&state, 7, 1, &actor, &clock).await })
    }
}

#[tokio::test]
async fn native_studio_automatic_two_member_receive_events_pacing_and_restart() {
    let hub = Hub::new();
    let mut alice = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(1000)),
        "alice",
    )
    .unwrap();
    alice.subscribe_control().await.unwrap();
    let invite = alice.mint_invite([1; 16], u64::MAX, vec![]).unwrap();
    let (bob, tick) = tokio::join!(
        Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(ManualClock::new(1000)),
            "bob",
            alice.local_peer(),
            &invite
        ),
        alice.sync_once()
    );
    tick.unwrap();
    let mut a = Peer::new(alice).await;
    let mut b = Peer::new(bob.unwrap()).await;
    invoke(
        &b.state,
        7,
        StudioRequest::Read {
            target: StudioTarget::Index {
                channel: channel_id(&channel()).unwrap(),
            },
        },
    )
    .await
    .unwrap();
    assert!(
        invoke(&b.state, 7, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .is_none()
    );
    invoke(&a.state, 7, create()).await.unwrap();
    let mut pending = b.actor.studio_pending();
    pending.wait_for(|p| *p).await.unwrap(); // already-true work must run on worker startup
    let clock = PaceClock::new();
    let b_state = b.state.clone();
    let b_actor = b.actor.clone();
    let mut driver = Box::pin(drive_receiver(&b_state, 7, 1, &b_actor, &clock));
    // Watch order, not packet arrival, selects the next document; both Create records must
    // arrive exactly once, without imposing cross-document delivery order.
    let first = while_driving(&mut driver, b.updates.recv()).await.unwrap();
    while_driving(&mut driver, clock.sleeping.notified()).await;
    // Index packet is still queued. Neither an unchanged true level nor fresh traffic can
    // cause a second receive before this injected deadline; no event is delivered prematurely.
    assert!(b.updates.try_recv().is_err());
    clock.inner.advance_ms(999);
    assert!(b.updates.try_recv().is_err());
    clock.inner.advance_ms(1);
    let second = while_driving(&mut driver, b.updates.recv()).await.unwrap();
    assert_ne!(first, second);
    assert!([first, second].contains(&None));
    assert!([first, second].contains(&Some([7; 16])));
    while_driving(&mut driver, clock.sleeping.notified()).await;
    let mut pix = vec![
        80, 73, 88, 49, 191, 143, 3, 1, 19, 18, 24, 2, 232, 230, 240, 3, 151, 125, 242, 0, 224,
        122, 184,
    ];
    pix.extend((0..108).flat_map(|_| [255, 0]));
    let cid = a.actor.publish_pix(pix.clone(), None).await.unwrap().cid;
    let created = invoke_read(&a.state).await;
    let body = FlipnoteOp::InsertFrame {
        frame: [4; 16],
        after: None,
        cid: *Cid::from_hex(&cid).unwrap().as_bytes(),
        bytes: pix.len() as u64,
    }
    .encode()
    .unwrap();
    invoke(
        &a.state,
        7,
        apply_request(
            &channel(),
            Some(&hex::encode([7; 16])),
            created["epochId"].as_str().unwrap(),
            &hex::encode([8; 16]),
            String::from_utf8(body).unwrap(),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    pending.wait_for(|p| *p).await.unwrap();
    // The second Create packet emptied the inbox; the new frame changed false -> true while
    // the worker was sleeping. Poll the REAL worker deterministically before/at 999ms: even
    // native operation-slot acquisition must not start early (the old select failed this).
    for elapsed in [0, 999] {
        clock.inner.advance_ms(elapsed);
        std::future::poll_fn(|cx| {
            assert!(driver.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(b.state.inline_downloads.lock().unwrap().is_empty());
        assert!(b.updates.try_recv().is_err());
    }
    clock.inner.advance_ms(1);
    assert_eq!(
        while_driving(&mut driver, b.updates.recv()).await.unwrap(),
        Some([7; 16])
    );
    while_driving(&mut driver, clock.sleeping.notified()).await;
    let received = invoke_read(&b.state).await;
    assert_eq!(
        received["content"]["frames"][hex::encode([4; 16])]["pixels"]["selected"]["value"]["cid"],
        cid
    );
    // Receiving metadata does not fetch the pixel blob or claim byte possession.
    assert!(b
        .state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .blob_store(&hex::encode(&b.state.servers.lock().await[&7].group_id))
        .unwrap()
        .get_bounded(&Cid::from_hex(&cid).unwrap(), pix.len())
        .unwrap()
        .is_none());
    while a.updates.try_recv().is_ok() {}
    let title = FlipnoteOp::SetHeader(FlipnoteHeader::Title("bob drew this".into()))
        .encode()
        .unwrap();
    invoke(
        &b.state,
        7,
        apply_request(
            &channel(),
            Some(&hex::encode([7; 16])),
            received["epochId"].as_str().unwrap(),
            &hex::encode([9; 16]),
            String::from_utf8(title).unwrap(),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let a_clock = PaceClock::new();
    let a_driver = a.driver(a_clock.clone());
    assert_eq!(a.updates.recv().await.unwrap(), Some([7; 16]));
    a_clock.sleeping.notified().await;
    assert_eq!(
        invoke_read(&a.state).await["content"]["title"]["selected"]["value"],
        "bob drew this"
    );
    drop(driver);
    a_driver.abort();
    assert!(a_driver.await.unwrap_err().is_cancelled());
    a.actor.shutdown().await;
    b.actor.shutdown().await;
    a.task.await.unwrap();
    b.task.await.unwrap();
    a.drain.await.unwrap();
    b.drain.await.unwrap();
    let snapshot = b
        .state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .unwrap();
    let restored = Server::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(3)),
        rng(),
        Box::new(ManualClock::new(5000)),
        "bob",
    )
    .unwrap();
    let group = restored.group_id();
    let device = restored.device_id();
    let (actor, mut events, task) = spawn(restored);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    install(&b.state, actor.clone(), group, device, 2).await;
    assert_eq!(
        invoke_read(&b.state).await["content"]["title"]["selected"]["value"],
        "bob drew this"
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_studio_receiver_and_delayed_events_refuse_old_incarnation_and_lock() {
    let (_root, state, actor, task, drain) = fixture().await;
    state.servers.lock().await.get_mut(&7).unwrap().instance = 2;
    assert!(receive_once(&state, 7, 1, &actor).await.is_err());
    assert!(!forward_if_current(&state, 7, 1, || panic!("old actor event escaped")).await);
    assert!(forward_if_current(&state, 7, 2, || {}).await);
    state.session_lock_requested.store(true, Ordering::Release);
    assert!(receive_once(&state, 7, 2, &actor).await.is_err());
    assert!(!forward_if_current(&state, 7, 2, || panic!("locked event escaped")).await);
    assert!(state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .is_err());
    assert!(state.ui_session_commit.try_lock().is_ok());
    assert!(persist_lock_for(&state, 7).try_lock_owned().is_ok());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}
