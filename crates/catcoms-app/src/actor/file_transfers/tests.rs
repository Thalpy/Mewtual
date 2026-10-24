use super::*;
use async_trait::async_trait;
use bytes::Bytes;
use catcoms_mls::MlsDevice;
use catcoms_rt::{
    Hub, ManualClock, MemNetwork, PeerConnectionSnapshot, ProtocolId, Topic, TransportError,
    TransportEvent,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

/// Delay only the new blob seam. Membership/catch-up still use real authenticated in-memory
/// exchanges, so these regressions exercise the actor and verification paths, not a fake server.
struct GatedTransport {
    inner: MemNetwork,
    gate: Arc<Semaphore>,
    entered: mpsc::UnboundedSender<RequestCancellation>,
}

#[async_trait]
impl MeshTransport for GatedTransport {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        self.inner.connection_snapshot()
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
    async fn request(&self, p: PeerId, t: ProtocolId, b: Bytes) -> Result<Bytes, TransportError> {
        self.inner.request(p, t, b).await
    }
    async fn request_cancellable(
        &self,
        p: PeerId,
        t: ProtocolId,
        b: Bytes,
        c: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.inner.request_cancellable(p, t, b, c).await
    }
    async fn request_connected_cancellable(
        &self,
        p: PeerId,
        t: ProtocolId,
        b: Bytes,
        mut c: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.entered.send(c.clone()).unwrap();
        tokio::select! {
            biased;
            _ = c.cancelled() => return Err(TransportError::Cancelled),
            permit = self.gate.acquire() => { permit.unwrap().forget(); }
        }
        self.inner.request_connected_cancellable(p, t, b, c).await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        self.inner.next_event().await
    }
}

struct Pair {
    requester: Server<GatedTransport, ChaCha20Rng>,
    holder: Server<MemNetwork, ChaCha20Rng>,
    cid: Cid,
    data: Vec<u8>,
    gate: Arc<Semaphore>,
    entered: mpsc::UnboundedReceiver<RequestCancellation>,
    clock: ManualClock,
}

async fn pair() -> Pair {
    bounded(pair_inner()).await
}

async fn pair_inner() -> Pair {
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let gate = Arc::new(Semaphore::new(0));
    let (entered, received) = mpsc::unbounded_channel();
    let holder_peer = PeerId::from_u64(1);
    let mut holder = Server::found(
        hub.join(holder_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1),
        Box::new(clock.clone()),
        "alice",
    )
    .unwrap();
    holder.subscribe_control().await.unwrap();
    holder.open_files().await.unwrap();
    let data = b"a file that must not pin the actor".to_vec();
    let cid = holder
        .add_file("test.bin", "application/octet-stream", "", &data)
        .await
        .unwrap();
    let invite = holder.mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let mut requester = {
        let joining = Server::join(
            GatedTransport {
                inner: hub.join(PeerId::from_u64(2)),
                gate: gate.clone(),
                entered,
            },
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(clock.clone()),
            "bob",
            holder_peer,
            &invite,
        );
        tokio::pin!(joining);
        loop {
            tokio::select! { result = &mut joining => break result.unwrap(), _ = holder.sync_once() => {} }
        }
    };
    requester.open_files().await.unwrap();
    {
        let catchup = requester.request_files_catchup(holder_peer);
        tokio::pin!(catchup);
        loop {
            tokio::select! { result = &mut catchup => { result.unwrap(); break; }, _ = holder.sync_once() => {} }
        }
    }
    assert!(requester.sync.blob_cids().is_empty());
    assert_eq!(requester.sync.blob_fetch_peers(), vec![holder_peer]);
    Pair {
        requester,
        holder,
        cid,
        data,
        gate,
        entered: received,
        clock,
    }
}

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("actor operation stalled")
}

#[tokio::test]
async fn a_withholding_chunk_does_not_block_commands_or_another_local_read() {
    let Pair {
        mut requester,
        holder,
        cid,
        data,
        gate,
        mut entered,
        ..
    } = pair().await;
    let local = requester
        .add_file("local.txt", "text/plain", "", b"local")
        .await
        .unwrap();
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, _events, task) = spawn(requester);
    let download = tokio::spawn({
        let actor = actor.clone();
        async move { actor.fetch_file_chunk(cid.as_bytes().to_vec(), 0).await }
    });
    let attempt = bounded(entered.recv()).await.unwrap();
    assert!(!attempt.is_cancelled());
    assert!(bounded(actor.file_head(cid.as_bytes().to_vec()))
        .await
        .is_some());
    assert_eq!(
        bounded(actor.fetch_file_chunk(local.as_bytes().to_vec(), 0))
            .await
            .unwrap()
            .0,
        b"local"
    );
    gate.add_permits(1);
    assert_eq!(bounded(download).await.unwrap().unwrap().0, data);
    actor.shutdown().await;
    source.shutdown().await;
    bounded(task).await.unwrap();
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn unlisting_during_network_wait_rejects_late_bytes_before_storage() {
    let Pair {
        requester,
        holder,
        cid,
        gate,
        mut entered,
        ..
    } = pair().await;
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, _events, task) = spawn(requester);
    let download = tokio::spawn({
        let actor = actor.clone();
        async move { actor.fetch_file_chunk(cid.as_bytes().to_vec(), 0).await }
    });
    let _attempt = bounded(entered.recv()).await.unwrap();
    bounded(source.delete_file(cid.as_bytes().to_vec()))
        .await
        .unwrap();
    bounded(async {
        loop {
            if actor.file_head(cid.as_bytes().to_vec()).await.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    gate.add_permits(1);
    assert!(bounded(download)
        .await
        .unwrap()
        .unwrap_err()
        .contains("authorization changed"));
    assert!(!bounded(actor.file_available(cid.as_bytes().to_vec())).await);
    actor.shutdown().await;
    source.shutdown().await;
    bounded(task).await.unwrap();
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn dropped_range_reply_and_shutdown_signal_pending_attempts() {
    for shutdown in [false, true] {
        let Pair {
            requester,
            holder,
            cid,
            mut entered,
            ..
        } = pair().await;
        let version = requester.file_head(&cid).unwrap().manifest_version;
        let (source, _source_events, source_task) = spawn(holder);
        let (actor, _events, task) = spawn(requester);
        let download = tokio::spawn({
            let actor = actor.clone();
            async move {
                actor
                    .read_file_range(cid.as_bytes().to_vec(), version, 0, 32)
                    .await
            }
        });
        let mut attempt = bounded(entered.recv()).await.unwrap();
        if shutdown {
            actor.shutdown().await;
            bounded(task).await.unwrap();
        } else {
            download.abort();
            bounded(attempt.cancelled()).await;
            actor.shutdown().await;
            bounded(task).await.unwrap();
        }
        bounded(attempt.cancelled()).await;
        assert!(
            attempt.is_cancelled(),
            "queued lower request must observe true, not only sender closure"
        );
        download.abort();
        source.shutdown().await;
        bounded(source_task).await.unwrap();
    }
}

#[tokio::test]
async fn range_limit_is_checked_before_any_network_request() {
    let Pair {
        requester,
        holder: _,
        cid,
        mut entered,
        ..
    } = pair().await;
    let version = requester.file_head(&cid).unwrap().manifest_version;
    let (actor, _events, task) = spawn(requester);
    assert!(
        bounded(actor.read_file_range(cid.as_bytes().to_vec(), version, 0, CHUNK_BYTES + 1))
            .await
            .unwrap_err()
            .contains("window limit")
    );
    assert!(entered.try_recv().is_err());
    actor.shutdown().await;
    bounded(task).await.unwrap();
}

#[tokio::test]
async fn cancelled_or_timed_out_requests_hold_capacity_until_transport_retirement() {
    let Pair {
        mut requester,
        holder: _,
        cid,
        mut entered,
        clock,
        ..
    } = pair().await;
    // A per-test process pool avoids cross-test timing changing this capacity assertion.
    let mut transfers = FileTransfers {
        running: JoinSet::new(),
        slots: Arc::new(Semaphore::new(1)),
        process: Arc::new(Semaphore::new(1)),
    };
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let attempt = bounded(entered.recv()).await.unwrap();
    clock.advance_ms(ATTEMPT_MS);
    let finished = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, finished);
    assert!(bounded(result).await.unwrap().is_err());
    assert!(attempt.is_cancelled());
    assert_eq!(
        transfers.slots.available_permits(),
        0,
        "lower ownership has not retired"
    );
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    assert!(bounded(result).await.unwrap().unwrap_err().contains("busy"));
    drop(attempt);
    assert_eq!(transfers.slots.available_permits(), 1);
}

fn attach_kept<T: MeshTransport>(server: &mut Server<T, ChaCha20Rng>, root: &std::path::Path) {
    use catcoms_storage::BlobStore;
    let mut primary = catcoms_storage::SealingBlobStore::open(
        root.join("cache"),
        [5; 32],
        ChaCha20Rng::seed_from_u64(90),
    )
    .unwrap();
    for cid in server.sync.blob_cids() {
        primary.put(&server.sync.get_blob(&cid).unwrap()).unwrap();
    }
    server.set_blob_store(Box::new(catcoms_storage::kept::KeptBlobStore::open(
        Box::new(primary),
        root.join("kept"),
        [6; 32],
        ChaCha20Rng::seed_from_u64(91),
    )));
}

#[tokio::test]
async fn kept_download_bypasses_cache_and_survives_unlisting_and_recheck() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        entered: _entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    gate.add_permits(8);
    let mut transfers = FileTransfers::new();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    for _ in 0..2 {
        let step = bounded(transfers.next()).await.unwrap();
        transfers.complete(&mut requester, step);
    }
    bounded(result).await.unwrap().unwrap();
    assert!(requester.sync.kept_files().files[0].checked);
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache")).unwrap().count(),
        0,
        "retained traffic leaves no uncapped cache bytes"
    );
    assert!(requester.file_available(&cid));
    // Mutate only the application index. The retained manifest has independent local ownership.
    requester
        .sync
        .post(crate::DocType::FileIndex, crate::FILE_INDEX_DOC, |doc| {
            crate::delete_file_entry(doc, cid.as_bytes(), None)
        })
        .await
        .unwrap();
    assert!(
        requester.file_head(&cid).is_none(),
        "local ownership does not authorize media"
    );
    source.shutdown().await;
    bounded(source_task).await.unwrap();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    bounded(result).await.unwrap().unwrap();
    assert!(requester.sync.kept_files().files[0].checked);
    assert!(requester.file_head(&cid).is_none());
    requester.sync.forget_kept(&cid).unwrap();
    assert!(requester.sync.blob_cids().is_empty());
}

#[tokio::test]
async fn cancelled_kept_wait_releases_reservation_without_storing_late_bytes() {
    let Pair {
        mut requester,
        holder: _,
        cid,
        mut entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let mut transfers = FileTransfers::new();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    let mut attempt = bounded(entered.recv()).await.unwrap();
    assert!(requester.sync.kept_files().allocated_bytes > 0);
    drop(result);
    bounded(attempt.cancelled()).await;
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    assert!(requester.sync.kept_files().files.is_empty());
    assert_eq!(requester.sync.kept_files().allocated_bytes, 0);
    assert!(requester.sync.blob_cids().is_empty());
}

#[tokio::test]
async fn cancelled_between_local_keep_chunks_does_not_finish_or_block_a_command() {
    let Pair { mut holder, .. } = pair().await;
    let bytes = vec![3; CHUNK_BYTES + 1];
    let cid = holder
        .add_file("two.bin", "application/octet-stream", "", &bytes)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut holder, dir.path());
    let mut transfers = FileTransfers::new();
    let (signal, receiver) = watch::channel(false);
    let (reply, result) = oneshot::channel();
    transfers.keep(
        &mut holder,
        cid.as_bytes().to_vec(),
        Some(RequestCancellation::new(receiver, None)),
        reply,
    );
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut holder, step);
    assert!(
        holder.sync.kept_files().files.is_empty(),
        "only one chunk committed in this actor step"
    );
    assert!(
        holder.file_head(&cid).is_some(),
        "actor state is accessible between chunks"
    );
    signal.send_replace(true);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut holder, step);
    assert!(bounded(result).await.unwrap().is_err());
    assert_eq!(holder.sync.kept_files().allocated_bytes, 0);
}

#[tokio::test]
async fn queued_commands_outrank_even_ready_local_keep_continuations() {
    let Pair {
        mut holder, cid, ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut holder, dir.path());
    let (actor, _events, task) = spawn(holder);
    let (keep_reply, kept) = oneshot::channel();
    let (view_reply, view) = oneshot::channel();
    // These bounded channel sends do not suspend on this single-thread executor, so both
    // commands are queued before the actor first runs. A local worker may become ready after
    // KeepFile, but must not jump ahead of the already-queued inventory and Shutdown commands.
    actor
        .cmd_tx
        .send(AppCommand::KeepFile {
            cid: cid.as_bytes().to_vec(),
            cancel: None,
            reply: keep_reply,
        })
        .await
        .unwrap();
    actor
        .cmd_tx
        .send(AppCommand::KeptFiles { reply: view_reply })
        .await
        .unwrap();
    actor.cmd_tx.send(AppCommand::Shutdown).await.unwrap();
    assert!(bounded(view).await.unwrap().files.is_empty());
    bounded(task).await.unwrap();
    assert!(bounded(kept).await.is_err());
    // Shutdown abandoned only a reserved non-serving directory; the next exclusive mount sweeps it.
    let reopened = catcoms_storage::kept::KeptBlobStore::open(
        Box::new(catcoms_storage::MemoryBlobStore::new()),
        dir.path().join("kept"),
        [6; 32],
        ChaCha20Rng::seed_from_u64(92),
    );
    assert!(catcoms_storage::BlobStore::kept_files(&reopened)
        .files
        .is_empty());
    assert_eq!(
        catcoms_storage::BlobStore::kept_files(&reopened).allocated_bytes,
        0
    );
}
