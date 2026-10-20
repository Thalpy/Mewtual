use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;
use tokio::sync::oneshot;

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|context| Poll::Ready(future.as_mut().poll(context))).await
}

/// Real mounted vault, actor and sealed upload chunks. Barriers control native admission and
/// publication timing; no supplied `locked` boolean stands in for the production session gate.
struct Fixture {
    root: tempfile::TempDir,
    state: Arc<AppState>,
    actor: ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: Option<tokio::task::JoinHandle<()>>,
    events: Option<tokio::sync::mpsc::Receiver<catcoms_app::TracedEvent>>,
    group: String,
}
impl Fixture {
    async fn new() -> Self {
        let mut fixture = Self::held_events().await;
        fixture.start_drain();
        fixture
    }
    async fn held_events() -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"upload session", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let mut server = Server::found(
            Hub::new().join(PeerId::from_u64(701)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(701),
            Box::new(ManualClock::new(1_000)),
            "uploader",
        )
        .unwrap();
        attach_blob_store(&state, &mut server).await;
        let group_id = server.group_id();
        let group = hex::encode(&group_id);
        let device = server.device_id();
        let (actor, events, task) = spawn(server);
        state.servers.lock().await.insert(
            1,
            crate::tests::snapshot_test_entry(70, actor.clone(), group_id, device),
        );
        Self {
            root,
            state,
            actor,
            task,
            drain: None,
            events: Some(events),
            group,
        }
    }
    fn start_drain(&mut self) {
        if let Some(mut events) = self.events.take() {
            self.drain = Some(tokio::spawn(async move {
                while events.recv().await.is_some() {}
            }));
        }
    }
    async fn upload(&self, id: &str, bytes: &[u8]) -> RegisteredUpload {
        let mut pending = PendingUpload {
            server: 1,
            upload_id: id.into(),
            mime: "application/octet-stream".into(),
            address: CidHasher::new(),
            buffer: Vec::new(),
            chunks: Vec::new(),
            sealing: None,
            bytes_seen: bytes.len() as u64,
            declared_size: bytes.len() as u64,
            chunk_total: 1,
            touched_at: 1_000,
        };
        pending.address.update(bytes);
        pending.chunks.push(
            self.actor
                .seal_upload_chunk(bytes.to_vec(), pending.mime.clone())
                .await
                .unwrap(),
        );
        assert!(pending.is_complete());
        RegisteredUpload {
            pending,
            generation: self.state.ui_session_generation.load(Ordering::Acquire),
            instance: 70,
            actor: self.actor.clone(),
        }
    }
    fn staged(&self, chunk: &FileRef) -> std::path::PathBuf {
        self.root
            .path()
            .join("blobs")
            .join(&self.group)
            .join("staging")
            .join(chunk.ciphertext_cid.to_hex())
    }
    async fn stop(mut self) {
        self.start_drain();
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.unwrap().await.unwrap();
    }
}

async fn publish(upload: RegisteredUpload) -> Result<String, String> {
    let pending = upload.pending;
    let sealed = pending.chunks.clone();
    let result = upload
        .actor
        .publish_upload(
            pending.upload_id,
            pending.mime,
            String::new(),
            *pending.address.cid().as_bytes(),
            pending.declared_size,
            pending.chunks,
        )
        .await;
    if result.is_err() {
        upload.actor.discard_upload(sealed).await;
    }
    result
}

#[tokio::test]
async fn a_lock_before_the_index_post_ends_the_upload_instead_of_publishing_it() {
    let f = Fixture::new().await;
    let upload = f.upload("before-lock", b"original bytes").await;
    let staged = f.staged(&upload.chunks[0]);
    assert!(
        staged.exists(),
        "actual sealed chunk must exist before the race"
    );
    let key = (1, "token".into());
    f.state.uploads.lock().await.insert(key.clone(), upload);
    // Hold commit so take captures the original generation and waits. Register the real lock
    // request before releasing it: the final gate must reject without ever calling the actor.
    let commit = f.state.ui_session_commit.lock().await;
    let mut take = Box::pin(take(&f.state, &key));
    assert!(poll_once(take.as_mut()).await.is_pending());
    register_ui_lock_request(&f.state);
    drop(commit);
    assert!(take.await.is_err());
    assert!(f.actor.files_view().await.files.is_empty());
    assert!(!staged.exists(), "refusal cleans the actual staged bytes");
    assert!(f.state.uploads.lock().await.is_empty());
    lock_session_inner(&f.state, None).await.unwrap();
    f.stop().await;
}

#[tokio::test]
async fn a_complete_upload_publishes_and_an_incomplete_one_is_destroyed() {
    let f = Fixture::new().await;
    let key = (1, "token".into());
    let upload = f.upload("complete", b"bytes").await;
    f.state.uploads.lock().await.insert(key.clone(), upload);
    let admitted = take(&f.state, &key).await.unwrap();
    assert!(f.state.uploads.lock().await.is_empty());
    spawn_publication(admitted, publish, |result| async move { result })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.actor.files_view().await.files.len(), 1);
    assert!(
        take(&f.state, &key).await.is_err(),
        "cannot publish a removed token twice"
    );
    let mut incomplete = f.upload("short", b"short").await;
    let staged = f.staged(&incomplete.chunks[0]);
    incomplete.pending.declared_size += 1;
    f.state.uploads.lock().await.insert(key.clone(), incomplete);
    assert!(take(&f.state, &key)
        .await
        .unwrap_err()
        .contains("did not send every chunk"));
    assert!(!staged.exists());
    assert!(f.state.uploads.lock().await.is_empty());
    assert_eq!(f.actor.files_view().await.files.len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn locking_the_session_drains_every_upload_in_flight() {
    let f = Fixture::new().await;
    let mut staged = Vec::new();
    for token in ["a", "b"] {
        let upload = f.upload(token, token.as_bytes()).await;
        staged.push(f.staged(&upload.chunks[0]));
        f.state
            .uploads
            .lock()
            .await
            .insert((1, token.into()), upload);
    }
    lock_session_inner(&f.state, None).await.unwrap();
    assert!(f.state.uploads.lock().await.is_empty());
    assert!(!*f.state.session_resumable.lock().await);
    assert!(staged.iter().all(|path| !path.exists()));
    f.stop().await;
}

#[tokio::test]
async fn original_upload_generation_is_not_revived_by_unlock_or_a_wait_for_the_map() {
    let f = Fixture::new().await;
    let key = (1, "token".into());
    let upload = f.upload("old", b"old generation").await;
    let staged = f.staged(&upload.chunks[0]);
    f.state.uploads.lock().await.insert(key.clone(), upload);
    // Capture first, then wait at the final map acquisition under commit. This reproduces
    // the original stale-unlocked-answer race, including a completed lock/unlock round trip.
    let commit = f.state.ui_session_commit.lock().await;
    let mut take = Box::pin(take(&f.state, &key));
    assert!(poll_once(take.as_mut()).await.is_pending());
    let map = f.state.uploads.lock().await;
    drop(commit);
    assert!(poll_once(take.as_mut()).await.is_pending());
    let generation = register_ui_lock_request(&f.state);
    // The actual lock waits for commit, so a true native unlock cannot pass here. Simulate its
    // eventual authorized generation only to pin that equality of unlocked state is insufficient.
    f.state
        .ui_authorized_generation
        .store(generation, Ordering::Release);
    f.state
        .session_lock_requested
        .store(false, Ordering::Release);
    drop(map);
    assert!(take.await.is_err());
    assert!(!staged.exists());
    assert!(f.actor.files_view().await.files.is_empty());
    f.stop().await;
}

#[tokio::test]
async fn admitted_publication_keeps_lock_waiting_even_after_outer_waiter_is_dropped() {
    let mut f = Fixture::held_events().await;
    let key = (1, "token".into());
    let upload = f.upload("published", b"worker bytes").await;
    f.state.uploads.lock().await.insert(key.clone(), upload);
    let admitted = take(&f.state, &key).await.unwrap();
    let (entered, started) = oneshot::channel();
    let (finished, done) = oneshot::channel();
    let actor = f.actor.clone();
    let state = f.state.clone();
    assert!(f.actor.files_view().await.files.is_empty());
    // Fill the real bounded event sink, then queue one more event-producing command. Whether
    // it is blocked on this command's event or the previous command's snapshot wake, the actor
    // cannot reach the later PublishUpload until we drain events. No fake actor result is used.
    let mut sequence = 0;
    while f.events.as_ref().unwrap().capacity() > 0 {
        f.actor
            .set_server_name(format!("event barrier {sequence}"))
            .await
            .unwrap();
        sequence += 1;
    }
    let barrier_actor = f.actor.clone();
    let mut barrier = Box::pin(async move {
        barrier_actor
            .set_server_name("last event barrier".into())
            .await
    });
    assert!(poll_once(barrier.as_mut()).await.is_pending());
    let worker = spawn_publication(
        admitted,
        move |upload| async move {
            entered.send(()).unwrap();
            publish(upload).await
        },
        move |result| async move {
            assert!(result.is_ok());
            assert_eq!(
                persist_server_instance(&state, 1, 70, actor).await,
                PersistOutcome::Durable
            );
            finished.send(()).unwrap();
        },
    );
    started.await.unwrap();
    drop(worker); // dropped invoke waiter; actual worker and owned commit continue
    let locking_state = f.state.clone();
    let mut lock = Box::pin(lock_session_inner(&locking_state, None));
    assert!(
        poll_once(lock.as_mut()).await.is_pending(),
        "lock cannot finish before publication"
    );
    let observation_actor = f.actor.clone();
    let mut observed = Box::pin(async move { observation_actor.files_view().await });
    assert!(
        poll_once(observed.as_mut()).await.is_pending(),
        "real actor remains behind the event barrier"
    );
    f.start_drain();
    lock.await.unwrap();
    barrier.await.unwrap();
    assert_eq!(observed.await.files.len(), 1);
    assert_eq!(
        f.actor.files_view().await.files.len(),
        1,
        "publication precedes lock completion"
    );
    done.await.unwrap();
    let saved = f
        .state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(1)
        .unwrap();
    let restored = Server::restore(
        &saved,
        Hub::new().join(PeerId::from_u64(702)),
        ChaCha20Rng::seed_from_u64(702),
        Box::new(ManualClock::new(2_000)),
        "uploader",
    )
    .unwrap();
    let live = f.actor.files_view().await.files;
    assert_eq!(live[0].entry.name, "published");
    assert_eq!(live[0].entry.cid, Cid::of(b"worker bytes").as_bytes());
    assert_eq!(
        restored.files(),
        live.into_iter()
            .map(|listing| listing.entry)
            .collect::<Vec<_>>(),
        "the exact saved projection must include the accepted upload, not an older snapshot"
    );
    f.stop().await;
}

#[tokio::test]
async fn begin_waiting_on_the_upload_map_cannot_insert_after_lock_request() {
    let f = Fixture::new().await;
    let mut upload = f.upload("new", b"not yet reserved").await;
    // Begin's candidate has no sealed bytes yet; preserve that production invariant.
    f.actor
        .discard_upload(std::mem::take(&mut upload.pending.chunks))
        .await;
    let map = f.state.uploads.lock().await;
    let mut begin = Box::pin(register(&f.state, "new-token".into(), upload, 1_000));
    assert!(poll_once(begin.as_mut()).await.is_pending());
    register_ui_lock_request(&f.state);
    drop(map);
    assert!(begin.await.is_err());
    assert!(f.state.uploads.lock().await.is_empty());
    f.stop().await;
}

#[tokio::test]
async fn quota_refusal_cleans_retired_chunks_and_publish_failure_keeps_no_orphans() {
    let f = Fixture::new().await;
    let old = f.upload("old-id", b"stale staged bytes").await;
    let staged = f.staged(&old.chunks[0]);
    f.state
        .uploads
        .lock()
        .await
        .insert((1, "old-token".into()), old);
    // Keep the starting map within both production limits. Only the unrelated occupants'
    // chunk counts are synthetic, avoiding a 2 GiB test write; the retired/failing chunks are
    // genuinely sealed and their deletion is checked on disk.
    let chunks_per_file = upload_chunk_count(MAX_FILE_BYTES as u64);
    let mut remaining_chunks = (MAX_STAGED_UPLOAD_BYTES / CHUNK_BYTES as u64) as usize - 1;
    let mut index = 0;
    while remaining_chunks > 0 {
        let mut upload = f.upload(&format!("other-{index}"), b"other bytes").await;
        let count = remaining_chunks.min(chunks_per_file);
        let chunk = upload.chunks[0].clone();
        upload.pending.chunks.resize(count, chunk);
        upload.pending.chunk_total = count;
        upload.pending.declared_size = count as u64 * CHUNK_BYTES as u64;
        upload.pending.touched_at = u64::MAX;
        f.state
            .uploads
            .lock()
            .await
            .insert((1, format!("other-{index}")), upload);
        remaining_chunks -= count;
        index += 1;
    }
    assert!(f.state.uploads.lock().await.len() < MAX_PENDING_UPLOADS);
    let mut candidate = f.upload("replacement", b"candidate").await;
    f.actor
        .discard_upload(std::mem::take(&mut candidate.pending.chunks))
        .await;
    candidate.pending.declared_size = CHUNK_BYTES as u64 + 1;
    candidate.pending.chunk_total = 2;
    let error = register(
        &f.state,
        "replacement".into(),
        candidate,
        UPLOAD_IDLE_TIMEOUT_MS + 2_000,
    )
    .await
    .unwrap_err();
    assert!(error.contains("too much upload data"), "{error}");
    assert!(
        !staged.exists(),
        "quota refusal must still clean the retired real chunk"
    );
    // A separate invalid name reaches the real actor failure path after native admission.
    let key = (1, "bad-name".into());
    let mut upload = f.upload("bad", b"failed publication").await;
    upload.pending.upload_id = "x".repeat(10_000);
    let staged = f.staged(&upload.chunks[0]);
    f.state.uploads.lock().await.insert(key.clone(), upload);
    let admitted = take(&f.state, &key).await.unwrap();
    assert!(
        spawn_publication(admitted, publish, |result| async move { result })
            .await
            .unwrap()
            .is_err()
    );
    assert!(!staged.exists());
    assert!(f.actor.files_view().await.files.is_empty());
    lock_session_inner(&f.state, None).await.unwrap();
    f.stop().await;
}

#[tokio::test]
async fn a_completed_native_lock_unlock_cannot_authorize_an_old_begin() {
    let f = Fixture::new().await;
    let mut old = f.upload("old begin", b"old").await;
    f.actor
        .discard_upload(std::mem::take(&mut old.pending.chunks))
        .await;
    let old_generation = old.generation;
    lock_session_inner(&f.state, None).await.unwrap();
    let fresh_generation = f.state.ui_session_generation.load(Ordering::Acquire);
    finalize_unlock_session(&f.state, fresh_generation, true)
        .await
        .unwrap();
    assert_ne!(old_generation, fresh_generation);
    assert!(register(&f.state, "old-token".into(), old, 1_000)
        .await
        .is_err());
    assert!(f.state.uploads.lock().await.is_empty());
    let fresh = f.upload("fresh", b"new").await;
    assert_eq!(
        register(&f.state, "new-token".into(), fresh, 1_000)
            .await
            .unwrap(),
        0
    );
    lock_session_inner(&f.state, None).await.unwrap();
    f.stop().await;
}

#[tokio::test]
async fn publication_releases_commit_before_waiting_for_the_same_persist_lock_as_leave() {
    let f = Fixture::new().await;
    let key = (1, "token".into());
    let upload = f.upload("leave order", b"published before leave").await;
    f.state.uploads.lock().await.insert(key.clone(), upload);
    let admitted = take(&f.state, &key).await.unwrap();
    let generation = f.state.ui_session_generation.load(Ordering::Acquire);
    // These are leave_server's real two locks, in its production order. The worker must
    // release commit before its persistence stage waits for the first one.
    let writing = persist_lock_for(&f.state, 1).lock_owned().await;
    let (settled, reached_persistence) = oneshot::channel();
    let state = f.state.clone();
    let actor = f.actor.clone();
    let worker = spawn_publication(admitted, publish, move |result| async move {
        assert!(result.is_ok());
        settled.send(()).unwrap();
        persist_server_instance(&state, 1, 70, actor).await
    });
    reached_persistence.await.unwrap();
    // Test the shared mutex directly: registry/net persistence may briefly use other locks,
    // so requiring the entire permission check to finish in one poll would be a false oracle.
    let leave_commit = f
        .state
        .ui_session_commit
        .try_lock()
        .expect("publication held commit while waiting for leave's persistence lock");
    drop(leave_commit);
    drop(
        require_ui_session_generation(&f.state, generation)
            .await
            .unwrap(),
    );
    drop(writing);
    assert_eq!(worker.await.unwrap(), PersistOutcome::Durable);
    f.stop().await;
}

#[tokio::test]
async fn replaced_server_cannot_adopt_an_upload_or_redirect_its_cleanup() {
    let f = Fixture::new().await;
    let key = (1, "old-token".into());
    let upload = f
        .upload("original actor", b"original private staging")
        .await;
    let staged = f.staged(&upload.chunks[0]);
    f.state.uploads.lock().await.insert(key.clone(), upload);
    let replacement = Server::found(
        Hub::new().join(PeerId::from_u64(703)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(703),
        Box::new(ManualClock::new(1_000)),
        "replacement",
    )
    .unwrap();
    let group = replacement.group_id();
    let device = replacement.device_id();
    let (actor, mut events, task) = spawn(replacement);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    f.state.servers.lock().await.insert(
        1,
        crate::tests::snapshot_test_entry(71, actor.clone(), group, device),
    );
    assert!(take(&f.state, &key)
        .await
        .unwrap_err()
        .contains("server changed"));
    assert!(
        !staged.exists(),
        "cleanup must reach the original actor's actual staging store"
    );
    assert!(actor.files_view().await.files.is_empty());
    assert!(f.actor.files_view().await.files.is_empty());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    f.stop().await;
}
