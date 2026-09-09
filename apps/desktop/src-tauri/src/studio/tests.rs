use super::*;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::Future;
mod receiver;
mod recovery;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(81)
}
fn channel() -> String {
    catcoms_app::channel_id("general").to_string()
}
fn target() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: channel_id(&channel()).unwrap(),
        object: [7; 16],
    }
}
fn create() -> StudioRequest {
    StudioRequest::Create {
        channel: channel_id(&channel()).unwrap(),
        object: [7; 16],
        nonce: [9; 16],
        title: "moon cat".into(),
        ts: 123,
    }
}
async fn install(
    state: &AppState,
    actor: ServerActor,
    group_id: Vec<u8>,
    device_id: DeviceId,
    instance: u64,
) {
    state.servers.lock().await.insert(
        7,
        ServerEntry {
            actor,
            instance,
            group_id,
            device_id,
            invite: None,
            name: "test".into(),
            bootstrap: vec![],
            bootstrap_owners: HashMap::new(),
            interface_routes: None,
            rendezvous: vec![],
            mesh: None,
            is_dm: false,
            switchboard: false,
            record_seq: 0,
            persist: PersistCounters::default(),
        },
    );
}
async fn fixture() -> (
    tempfile::TempDir,
    Arc<AppState>,
    ServerActor,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::default());
    *state.store.lock().await =
        Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
    *state.session_resumable.lock().await = true;
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    attach_blob_store(&state, &mut server).await;
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, mut events, task) = spawn(server);
    install(&state, actor.clone(), group_id, device_id, 1).await;
    let s = state.clone();
    // Model the real forwarder's vault-dependent session check. This used to be the other half
    // of the proposed queue-held-vault deadlock; the Ready handoff must coexist with it.
    let drain = tokio::spawn(async move {
        while events.recv().await.is_some() {
            let _ = require_unlocked_session(&s).await;
        }
    });
    (root, state, actor, task, drain)
}

#[tokio::test]
async fn native_studio_save_restart_reopen_real_blob_and_complete_projection() {
    let (root, state, actor, task, drain) = fixture().await;
    let created = invoke(&state, 7, create()).await.unwrap().unwrap();
    assert_eq!(created["publication"], "local");
    assert_eq!(created["provisional"], true);
    assert_eq!(created["phase"], "open");
    let mut pix = vec![
        80, 73, 88, 49, 191, 143, 3, 1, 19, 18, 24, 2, 232, 230, 240, 3, 151, 125, 242, 0, 224,
        122, 184,
    ];
    pix.extend((0..108).flat_map(|_| [255, 0]));
    let cid = actor.publish_pix(pix.clone(), None).await.unwrap().cid;
    let body = FlipnoteOp::InsertFrame {
        frame: [1; 16],
        after: None,
        cid: *Cid::from_hex(&cid).unwrap().as_bytes(),
        bytes: pix.len() as u64,
    }
    .encode()
    .unwrap();
    let request = || {
        apply_request(
            &channel(),
            Some(&hex::encode([7; 16])),
            created["epochId"].as_str().unwrap(),
            &hex::encode([8; 16]),
            String::from_utf8(body.clone()).unwrap(),
        )
        .unwrap()
    };
    let saved = invoke(&state, 7, request()).await.unwrap().unwrap();
    assert_eq!(
        saved["content"]["frames"][hex::encode([1; 16])]["pixels"]["selected"]["value"]["cid"],
        cid
    );
    assert_eq!(
        saved["content"]["frames"][hex::encode([1; 16])]["pixels"]["selected"]["source"]["author"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    let (group_id, device_id) = {
        let entries = state.servers.lock().await;
        let e = &entries[&7];
        (e.group_id.clone(), e.device_id)
    };
    state.servers.lock().await.clear();
    drop(state.store.lock().await.take());
    *state.store.lock().await =
        Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
    let snapshot = state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .unwrap();
    let mut server = Server::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(2)),
        rng(),
        Box::new(ManualClock::new(999)),
        "alice",
    )
    .unwrap();
    attach_blob_store(&state, &mut server).await;
    let (actor, mut events, task) = spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    install(&state, actor.clone(), group_id, device_id, 2).await;
    assert_eq!(
        invoke(&state, 7, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .unwrap(),
        saved
    );
    assert_eq!(invoke(&state, 7, request()).await.unwrap().unwrap(), saved);
    assert_eq!(
        actor
            .request_blob_bounded(Cid::from_hex(&cid).unwrap(), pix.len(), None)
            .await
            .unwrap()
            .unwrap(),
        pix
    );
    let index = invoke(
        &state,
        7,
        StudioRequest::Read {
            target: StudioTarget::Index {
                channel: channel_id(&channel()).unwrap(),
            },
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        index["content"]["objects"][hex::encode([7; 16])]["expiry"]["selected"]["value"],
        json!({"kind":"unrecorded"})
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_studio_ready_refuses_every_busy_or_stale_fence_without_waiting() {
    let (_root, state, actor, task, drain) = fixture().await;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    for gate in 0..5 {
        let ready = actor.studio_begin(create()).await.unwrap();
        match gate {
            0 => {
                let _guard = state.store.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            1 => {
                let _guard = state.ui_session_commit.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            2 => {
                let lock = persist_lock_for(&state, 7);
                let _guard = lock.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            3 => {
                let _guard = state.servers.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            _ => {
                let _guard = state.session_resumable.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
        }
        drop(ready);
        assert_eq!(actor.member_count().await, 1);
    }
    for mode in 0..3 {
        let ready = actor.studio_begin(create()).await.unwrap();
        // Each fence must reject independently, not inherit the stale generation
        // from the previous case and accidentally leave incarnation untested.
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        if mode == 0 {
            state.session_lock_requested.store(true, Ordering::Release);
        }
        if mode == 1 {
            state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        }
        if mode == 2 {
            state.servers.lock().await.get_mut(&7).unwrap().instance = 2;
        }
        assert!(authorize(&state, 7, 1, generation).is_err());
        drop(ready);
        state.session_lock_requested.store(false, Ordering::Release);
        assert_eq!(actor.member_count().await, 1);
    }
    assert!(
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(7)
            .is_err(),
        "no Studio snapshot write"
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_studio_lease_retains_fences_and_cancelled_receiver_cannot_create() {
    let (_root, state, actor, task, drain) = fixture().await;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    let ready = actor.studio_begin(create()).await.unwrap();
    let lease = authorize(&state, 7, 1, generation).unwrap();
    assert!(state.store.try_lock().is_err());
    assert!(state.servers.try_lock().is_err());
    assert!(state.ui_session_commit.try_lock().is_err());
    assert!(persist_lock_for(&state, 7).try_lock_owned().is_err());
    // Poll through lease transfer but cancel the result receiver before the single-threaded
    // actor runs. The command must release its owned native guards and perform no transaction.
    let mut future = Box::pin(ready.execute(lease));
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(future);
    assert_eq!(actor.member_count().await, 1);
    assert!(state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .is_err());
    assert!(state.servers.try_lock().is_ok());
    assert!(state.ui_session_commit.try_lock().is_ok());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[test]
fn native_studio_contract_rejects_bad_ids_bodies_and_preserves_expiry_states() {
    for bad in [
        "0",
        "ABABABABABABABABABABABABABABABAB",
        "gggggggggggggggggggggggggggggggg",
    ] {
        assert!(id(bad).is_err());
    }
    for bad in ["-1", "01", "+1", "340282366920938463463374607431768211456"] {
        assert!(channel_id(bad).is_err());
    }
    assert_eq!(
        expiry(StudioExpiry::Unrecorded),
        json!({"kind":"unrecorded"})
    );
    assert_eq!(expiry(StudioExpiry::Never), json!({"kind":"never"}));
    assert_eq!(expiry(StudioExpiry::At(0)), json!({"kind":"at","ms":0}));
    assert!(apply_request(
        &channel(),
        None,
        &"0".repeat(32),
        &"1".repeat(32),
        "x".repeat(65537)
    )
    .is_err());
    assert!(apply_request(
        &channel(),
        None,
        &"0".repeat(32),
        &"1".repeat(32),
        "{}".into()
    )
    .is_err());
    assert!(!format!("{:?}", create()).contains("moon cat"));
}

#[tokio::test]
async fn native_studio_cancelled_lease_releases_native_fences_without_a_save() {
    let (_root, state, actor, task, drain) = fixture().await;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    let ready = actor.studio_begin(create()).await.unwrap();
    let (slot, signal) = claim_internal_inline_download(&state).unwrap();
    let cancellation = RequestCancellation::new(signal, Some(slot.request_keepalive()));
    let lease = authorize(&state, 7, 1, generation)
        .unwrap()
        .with_cancellation(cancellation);
    // Native cancellation is the same live signal the actor now owns, not just a reply filter.
    cancel_all_inline_downloads(&state);
    assert!(ready.execute(lease).await.is_err());
    assert!(state.store.try_lock().is_ok());
    assert!(state.servers.try_lock().is_ok());
    assert!(state.ui_session_commit.try_lock().is_ok());
    assert!(state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .is_err());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

struct Pause {
    armed: AtomicBool,
    entered: StdMutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: StdMutex<bool>,
    wake: std::sync::Condvar,
}
struct PausingRng {
    inner: ChaCha20Rng,
    pause: Arc<Pause>,
}
// Never strand a blocking worker if an assertion fails while it is deliberately paused.
struct ReleaseOnDrop(Arc<Pause>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}
impl PausingRng {
    fn checkpoint(&self) {
        if self.pause.armed.swap(false, Ordering::AcqRel) {
            self.pause
                .entered
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            let mut released = self.pause.released.lock().unwrap();
            while !*released {
                released = self.pause.wake.wait(released).unwrap();
            }
        }
    }
}
impl rand_core::CryptoRng for PausingRng {}
impl rand_core::RngCore for PausingRng {
    fn next_u32(&mut self) -> u32 {
        self.checkpoint();
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.checkpoint();
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        self.checkpoint();
        self.inner.fill_bytes(out)
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(out);
        Ok(())
    }
}

#[tokio::test]
async fn native_studio_running_worker_retains_fences_after_invoke_or_actor_abort() {
    for abort_actor in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
        *state.session_resumable.lock().await = true;
        let (entered, entry) = tokio::sync::oneshot::channel();
        let pause = Arc::new(Pause {
            armed: AtomicBool::new(false),
            entered: StdMutex::new(Some(entered)),
            released: StdMutex::new(false),
            wake: std::sync::Condvar::new(),
        });
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            PausingRng {
                inner: rng(),
                pause: pause.clone(),
            },
            Box::new(ManualClock::new(123)),
            "alice",
        )
        .unwrap();
        let group = server.group_id();
        let device = server.device_id();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        install(&state, actor.clone(), group.clone(), device, 1).await;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let ready = actor.studio_begin(create()).await.unwrap();
        let lease = authorize(&state, 7, 1, generation).unwrap();
        pause.armed.store(true, Ordering::Release);
        let _release_on_drop = ReleaseOnDrop(pause.clone());
        let invoke = tokio::spawn(ready.execute(lease));
        entry.await.unwrap(); // Real worker paused at the first vault seal's injected RNG.
        invoke.abort();
        assert!(invoke.await.unwrap_err().is_cancelled());
        if abort_actor {
            task.abort();
        }
        state.session_lock_requested.store(true, Ordering::Release);
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        // Neither lock completion nor leave/reinstall nor another snapshot can pass its fence.
        assert!(state.store.try_lock().is_err());
        assert!(state.ui_session_commit.try_lock().is_err());
        assert!(state.servers.try_lock().is_err());
        assert!(persist_lock_for(&state, 7).try_lock_owned().is_err());
        *pause.released.lock().unwrap() = true;
        pause.wake.notify_all();
        let snapshot = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(7)
            .unwrap();
        if abort_actor {
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            actor.shutdown().await;
            task.await.unwrap();
        }
        drain.await.unwrap();
        assert!(state.ui_session_commit.try_lock().is_ok());
        assert!(state.servers.try_lock().is_ok());
        // A cancelled response is never shown, but an already-started save may finish. Reopen
        // the actual saved source with the durably saved MLS snapshot and verify the outcome.
        let server = Server::restore(
            &snapshot,
            Hub::new().join(PeerId::from_u64(2)),
            rng(),
            Box::new(ManualClock::new(999)),
            "alice",
        )
        .unwrap();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        install(&state, actor.clone(), group, device, 2).await;
        state.session_lock_requested.store(false, Ordering::Release);
        let reopened = invoke_read(&state).await;
        assert_eq!(
            reopened["content"]["title"]["selected"]["value"],
            "moon cat"
        );
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    }
}
async fn invoke_read(state: &AppState) -> Value {
    invoke(state, 7, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap()
}
