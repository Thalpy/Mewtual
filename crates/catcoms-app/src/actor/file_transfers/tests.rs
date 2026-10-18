use super::*;
use crate::{DocType, FileManifest};
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
    gated_peer: std::sync::Mutex<Option<PeerId>>,
    connection_events: std::sync::Mutex<VecDeque<PeerId>>,
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
        let applies = self.gated_peer.lock().unwrap().is_none_or(|peer| peer == p);
        if !applies {
            return self.inner.request_connected_cancellable(p, t, b, c).await;
        }
        self.entered.send(c.clone()).unwrap();
        tokio::select! {
            biased;
            _ = c.cancelled() => return Err(TransportError::Cancelled),
            permit = self.gate.acquire() => { permit.unwrap().forget(); }
        }
        self.inner.request_connected_cancellable(p, t, b, c).await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        if let Some(peer) = self.connection_events.lock().unwrap().pop_front() {
            return Some(TransportEvent::PeerConnected(peer));
        }
        self.inner.next_event().await
    }
}

struct Pair {
    hub: Arc<Hub>,
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
                gated_peer: std::sync::Mutex::new(None),
                connection_events: std::sync::Mutex::new(VecDeque::new()),
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
        hub,
        requester,
        holder,
        cid,
        data,
        gate,
        entered: received,
        clock,
    }
}

#[tokio::test]
async fn busy_preferred_provider_cannot_hide_a_later_connected_holder() {
    let Pair {
        hub,
        mut requester,
        mut holder,
        cid,
        data,
        mut entered,
        clock,
        ..
    } = pair().await;
    let preferred = holder.sync.local_peer();
    *requester.sync.transport().gated_peer.lock().unwrap() = Some(preferred);
    // The later route is a restored current-member source. Bootstrap candidates are deliberately
    // unproven endpoints: their signed blob answers still have to pass current membership.
    let mut later = Server::restore(
        &holder.snapshot().unwrap(),
        hub.join(PeerId::from_u64(3)),
        ChaCha20Rng::seed_from_u64(3),
        Box::new(clock.clone()),
        "restored holder",
    )
    .unwrap();
    for blob in holder.sync.blob_cids() {
        later
            .sync
            .put_blob(&holder.sync.get_blob(&blob).unwrap())
            .unwrap();
    }
    holder.set_blob_store(Box::new(catcoms_storage::MemoryBlobStore::new()));
    assert!(
        holder.sync.blob_cids().is_empty(),
        "the fifth provider is the sole ciphertext holder"
    );
    let mut empty_tasks = Vec::new();
    for number in 4..7 {
        let empty = hub.join(PeerId::from_u64(number));
        empty_tasks.push(tokio::spawn(async move {
            while let Some(event) = empty.next_event().await {
                if let TransportEvent::Request { responder, .. } = event {
                    responder.respond(Bytes::new());
                }
            }
        }));
    }
    // Hub registration is real connectivity but emits no connection edges. Supply those edges
    // through this wrapper's ordered event seam, independently of its real snapshot/requests.
    requester
        .sync
        .transport()
        .connection_events
        .lock()
        .unwrap()
        .extend((3..7).map(PeerId::from_u64));
    // The later route is fifth behind a preferred endpoint and three newer bootstrap peers.
    while requester.sync.stats().known_peers < 5 {
        bounded(requester.sync_once()).await.unwrap();
    }
    assert_eq!(requester.sync.blob_fetch_peers().len(), 4);
    assert!(!requester
        .sync
        .blob_fetch_peers()
        .contains(&later.sync.local_peer()));
    let (later_actor, _later_events, later_task) = spawn(later);
    let mut transfers = FileTransfers::new();
    let (held_reply, mut held_result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, held_reply);
    let _held_lower = bounded(entered.recv()).await.unwrap();
    assert!(transfers.inflight.contains(&preferred));
    let (reply, mut result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let received = bounded(async {
        loop {
            tokio::select! {
                value = &mut result => break value.unwrap().unwrap(),
                step = transfers.next() => transfers.complete(&mut requester, step.unwrap()),
            }
        }
    })
    .await;
    assert_eq!(received.0, data);
    assert!(
        transfers.inflight.contains(&preferred),
        "first provider remains genuinely occupied"
    );
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS - 1);
    assert!(
        held_result.try_recv().is_err(),
        "the first request never passed its gate"
    );
    drop(transfers);
    later_actor.cmd_tx.send(AppCommand::Shutdown).await.unwrap();
    bounded(later_task).await.unwrap();
    for task in empty_tasks {
        task.abort();
    }
}

#[tokio::test]
async fn a_bad_wrapped_key_does_not_reject_a_valid_reference_to_the_same_ciphertext() {
    let Pair {
        mut holder,
        cid,
        data,
        ..
    } = pair().await;
    add_bad_wrapped_variant(&mut holder, &cid).await;
    let mut transfers = FileTransfers::new();
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut holder, cid.as_bytes().to_vec(), 0, None, reply);
    assert_eq!(bounded(result).await.unwrap().unwrap().0, data);
    assert!(
        transfers.is_empty(),
        "valid local alternative needs no provider request"
    );
}

async fn add_bad_wrapped_variant<T: MeshTransport>(
    server: &mut Server<T, ChaCha20Rng>,
    cid: &Cid,
) -> FileManifest {
    let good = FileManifest::decode_or_legacy(&server.files()[0].file_ref).unwrap();
    let good_version = crate::file_manifest_version(&good.encode());
    let mut bad = good.clone();
    // Put the invalid key first in the real deterministic variant order, not merely row order.
    for nonce in 1u16..=u16::MAX {
        bad.chunks[0].wrapped_key.nonce[..2].copy_from_slice(&nonce.to_be_bytes());
        if crate::file_manifest_version(&bad.encode()) < good_version {
            break;
        }
    }
    assert!(crate::file_manifest_version(&bad.encode()) < good_version);
    assert_eq!(bad.chunks[0].ciphertext_cid, good.chunks[0].ciphertext_cid);
    let encoded = bad.encode();
    server
        .sync
        .post(DocType::FileIndex, 0, |doc| {
            crate::write_file_entry(
                doc,
                "bad.bin",
                "legacy",
                "",
                &encoded,
                crate::FileExpiry::Never,
                None,
            )
        })
        .await
        .unwrap();
    let resolved = crate::file_resolution::resolve(&server.files(), cid).unwrap();
    assert_eq!(resolved.variants.len(), 2);
    good
}

#[tokio::test]
async fn exact_bad_reference_stays_rejected_across_network_retry_rounds() {
    let Pair {
        mut requester,
        holder,
        cid,
        data,
        gate,
        mut entered,
        clock,
        ..
    } = pair().await;
    let good = add_bad_wrapped_variant(&mut requester, &cid).await;
    let (source, _events, source_task) = spawn(holder);
    let mut transfers = FileTransfers::new();
    let (reply, result) = oneshot::channel();
    gate.add_permits(1);
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let first = bounded(entered.recv()).await.unwrap();
    let first = {
        drop(first);
        bounded(transfers.next()).await.unwrap()
    };
    let TransferStep::Network(first_attempt) = &first else {
        panic!("expected first provider attempt");
    };
    assert_ne!(
        first_attempt.reference.encode(),
        good.chunks[0].encode(),
        "invalid key is attempted first"
    );
    transfers.complete(&mut requester, first);
    let timed_out = bounded(entered.recv()).await.unwrap();
    clock.advance_ms(ATTEMPT_MS);
    let step = bounded(transfers.next()).await.unwrap();
    let TransferStep::Network(attempt) = &step else {
        panic!("expected valid reference's timed-out attempt");
    };
    assert_eq!(attempt.reference.encode(), good.chunks[0].encode());
    assert_eq!(attempt.job.rejected_variants.len(), 1);
    transfers.complete(&mut requester, step);
    drop(timed_out);
    gate.add_permits(1);
    let resumed = resume_paused(&mut transfers, &clock, ROUND_BACKOFF_MS).await;
    transfers.complete(&mut requester, resumed);
    let lower = bounded(entered.recv()).await.unwrap();
    let completed = bounded(transfers.next()).await.unwrap();
    let TransferStep::Network(attempt) = &completed else {
        panic!("expected resumed provider attempt");
    };
    assert_eq!(
        attempt.reference.encode(),
        good.chunks[0].encode(),
        "retry does not reconsider the exact bad wrapped key"
    );
    transfers.complete(&mut requester, completed);
    drop(lower);
    assert_eq!(bounded(result).await.unwrap().unwrap().0, data);
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("actor operation stalled")
}

/// Resume a transfer step that is parked on a `ManualClock` sleep.
///
/// A paused retry round registers its sleep only when the spawned task is first polled, and the
/// test has no event to observe for that, so a single `advance_ms` can land before the target is
/// computed and be lost. Advancing repeatedly while re-polling removes the race without the test
/// having to guess at scheduler order.
async fn resume_paused(
    transfers: &mut FileTransfers,
    clock: &ManualClock,
    step_ms: u64,
) -> TransferStep {
    for _ in 0..64 {
        tokio::task::yield_now().await;
        clock.advance_ms(step_ms);
        if let Ok(Some(step)) =
            tokio::time::timeout(Duration::from_millis(20), transfers.next()).await
        {
            return step;
        }
    }
    panic!("a paused transfer round never resumed");
}

#[tokio::test]
async fn a_transport_failure_retries_the_chunk_against_the_only_provider() {
    // The deployed shape: one other member online, one encrypted variant, so the candidate list
    // holds exactly one entry. Draining it used to end the read, which meant a single dropped
    // response failed the whole download and `READ_MS` could never be spent. The first attempt
    // here expires against its own budget; the read must survive it and succeed on a later round.
    let Pair {
        mut requester,
        mut holder,
        cid,
        data,
        gate,
        mut entered,
        clock,
        ..
    } = pair().await;
    let mut transfers = FileTransfers::new();
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);

    // Attempt one is parked at the gate and expires against `ATTEMPT_MS`.
    let first = bounded(entered.recv()).await.unwrap();
    clock.advance_ms(ATTEMPT_MS);
    let expired = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, expired);
    drop(first);

    // The read is still open: a paused retry round, not a failure.
    assert!(
        !transfers.is_empty(),
        "an expired attempt must arm a retry rather than end the read"
    );

    // Let the next attempt through and drive the round to completion.
    gate.add_permits(1);
    let resumed = resume_paused(&mut transfers, &clock, ROUND_BACKOFF_MS).await;
    transfers.complete(&mut requester, resumed);
    let served = {
        let next = transfers.next();
        tokio::pin!(next);
        loop {
            tokio::select! {
                step = &mut next => break step.unwrap(),
                _ = holder.sync_once() => {}
            }
        }
    };
    transfers.complete(&mut requester, served);
    let (bytes, _) = bounded(result).await.unwrap().unwrap();
    assert_eq!(bytes, data, "the retried round must deliver the real chunk");
}

#[tokio::test]
async fn a_provider_carries_one_bulk_request_at_a_time() {
    // Two reads of the same chunk share the single connected provider. On a NAT-punched path
    // that provider is the only route there is, so the second read must wait for the first
    // rather than putting a second multi-megabyte request on the same wire.
    let Pair {
        mut requester,
        holder: _holder,
        cid,
        mut entered,
        ..
    } = pair().await;
    let mut transfers = FileTransfers::new();
    let (first_reply, _first_result) = oneshot::channel();
    transfers.chunk(
        &mut requester,
        cid.as_bytes().to_vec(),
        0,
        None,
        first_reply,
    );
    let _first = bounded(entered.recv()).await.unwrap();

    let (second_reply, _second_result) = oneshot::channel();
    transfers.chunk(
        &mut requester,
        cid.as_bytes().to_vec(),
        0,
        None,
        second_reply,
    );
    // The provider is busy, so the second read parks instead of reaching the transport. A
    // request would have arrived on `entered` immediately if it had been admitted.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), entered.recv())
            .await
            .is_err(),
        "a second bulk request reached the same provider while one was already in flight"
    );
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
        holder: _holder,
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
        holder: _holder,
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
        inflight: HashSet::new(),
    };
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let attempt = bounded(entered.recv()).await.unwrap();
    clock.advance_ms(ATTEMPT_MS);
    let finished = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, finished);
    // An expired attempt budget now arms a retry round rather than ending the read, so the
    // caller is still waiting here. Capacity is the thing under test and is unchanged: the
    // retry has to find its own permit, it cannot inherit the one the live lower request holds.
    assert!(attempt.is_cancelled());
    assert_eq!(
        transfers.slots.available_permits(),
        0,
        "lower ownership has not retired"
    );
    // Drive the paused round. The permit is still held by the unretired request, so the retry
    // is refused for capacity, which is what finally fails the read.
    let resumed = resume_paused(&mut transfers, &clock, ROUND_BACKOFF_MS).await;
    transfers.complete(&mut requester, resumed);
    assert!(bounded(result).await.unwrap().is_err());
    assert_eq!(
        transfers.slots.available_permits(),
        0,
        "a refused retry must not refund the live request's permit"
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
        holder: _holder,
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
