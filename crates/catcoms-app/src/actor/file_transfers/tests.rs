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

mod bridge;

/// These manually driven unit owners share production limits within each test, not with
/// unrelated parallel tests in this executable. Actual actor/transport integration retains the
/// process-global pool; capacity/retirement regressions below also choose explicit private pools.
fn isolated_transfers() -> FileTransfers {
    let mut transfers = FileTransfers::new();
    transfers.process = Arc::new(Semaphore::new(PROCESS_ATTEMPTS));
    transfers.process_jobs = Arc::new(Semaphore::new(PROCESS_ATTEMPTS));
    transfers
}

#[tokio::test]
async fn whole_job_admission_bounds_busy_peer_waiters_and_lower_credit_outlives_retired_job() {
    let Pair {
        mut requester,
        holder: _holder,
        cid,
        mut entered,
        ..
    } = pair().await;
    let mut transfers = isolated_transfers();
    let mut receivers = Vec::new();
    for _ in 0..SERVER_ATTEMPTS {
        let (reply, receive) = oneshot::channel();
        transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
        receivers.push(receive);
    }
    let lower = bounded(entered.recv()).await.unwrap();
    assert_eq!(transfers.jobs.available_permits(), 0);
    assert_eq!(
        transfers.running.len(),
        SERVER_ATTEMPTS,
        "one network plus three busy-peer waits"
    );
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS - 1);
    let (reply, rejected) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    assert!(bounded(rejected)
        .await
        .unwrap()
        .unwrap_err()
        .contains("busy"));
    assert_eq!(transfers.running.len(), SERVER_ATTEMPTS);

    drop(receivers.remove(0));
    let finished = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, finished);
    assert_eq!(
        transfers.jobs.available_permits(),
        1,
        "terminal job releases only its own whole-job owner"
    );
    assert_eq!(
        transfers.slots.available_permits(),
        SERVER_ATTEMPTS - 1,
        "actual lower custody is independent"
    );
    assert!(lower.is_cancelled());
    drop(lower);
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS);
    transfers.running.abort_all();
    bounded(async { while transfers.running.join_next().await.is_some() {} }).await;
    assert_eq!(transfers.jobs.available_permits(), SERVER_ATTEMPTS);
    assert_eq!(transfers.process_jobs.available_permits(), PROCESS_ATTEMPTS);
}

#[tokio::test]
async fn process_job_capacity_is_atomic_and_keep_refuses_before_a_run_or_durable_debit() {
    let Pair {
        mut requester, cid, ..
    } = pair().await;
    let directory = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, directory.path());
    let process = Arc::new(Semaphore::new(PROCESS_ATTEMPTS));
    let mut first = isolated_transfers();
    first.process_jobs = process.clone();
    let mut second = isolated_transfers();
    second.process_jobs = process.clone();
    let mut third = isolated_transfers();
    third.process_jobs = process.clone();
    let first_jobs: Vec<_> = (0..SERVER_ATTEMPTS)
        .map(|_| first.admit_job().unwrap())
        .collect();
    let second_jobs: Vec<_> = (0..SERVER_ATTEMPTS)
        .map(|_| second.admit_job().unwrap())
        .collect();
    assert!(first.admit_job().is_none());
    assert!(third.admit_job().is_none());
    assert_eq!(
        third.jobs.available_permits(),
        SERVER_ATTEMPTS,
        "failed global admission refunds unstarted local slot"
    );
    let (reply, result) = oneshot::channel();
    third.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    assert!(bounded(result).await.unwrap().unwrap_err().contains("busy"));
    let inventory = requester.sync.kept_files();
    assert!(
        inventory.files.is_empty() && inventory.transfers.is_empty(),
        "busy must precede starting/publishing Keep"
    );
    drop(first_jobs);
    let admitted = third.admit_job().unwrap();
    assert_eq!(process.available_permits(), SERVER_ATTEMPTS - 1);
    drop((admitted, second_jobs));
    assert_eq!(process.available_permits(), PROCESS_ATTEMPTS);
}

/// Delay only the new blob seam. Membership/catch-up still use real authenticated in-memory
/// exchanges, so these regressions exercise the actor and verification paths, not a fake server.
struct GatedTransport {
    inner: MemNetwork,
    gate: Arc<Semaphore>,
    entered: mpsc::UnboundedSender<RequestCancellation>,
    gated_peer: std::sync::Mutex<Option<PeerId>>,
    connection_events: std::sync::Mutex<VecDeque<PeerId>>,
    legacy_blob_only: std::sync::atomic::AtomicBool,
    sent_blob_requests: Arc<std::sync::atomic::AtomicUsize>,
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
        // A released provider predating kind27 returns the empty unsupported response. Keep
        // the actual kind5 exchange/authentication, rather than fabricating its completion.
        if self
            .legacy_blob_only
            .load(std::sync::atomic::Ordering::Acquire)
            && b.first() == Some(&27)
        {
            return Ok(Bytes::new());
        }
        // Mirror the driver's final synchronous admission check after a held command queue.
        // The cancellation object is the actual application-created owner, unchanged.
        if c.is_cancelled() {
            return Err(TransportError::Cancelled);
        }
        if matches!(b.first(), Some(5 | 27)) {
            self.sent_blob_requests
                .fetch_add(1, std::sync::atomic::Ordering::Release);
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
    clock_hook: ClockHook,
}

type ClockHook = Arc<std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>>;
/// One-shot native retirement on a real validation time sample, after CPU work has completed.
/// This tests ordering at the Clock seam without wall time, sleeping or a second executor.
struct ValidationClock {
    inner: ManualClock,
    hook: ClockHook,
}
impl std::fmt::Debug for ValidationClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ValidationClock { .. }")
    }
}
impl catcoms_rt::Clock for ValidationClock {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        let hook = self.hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        self.inner.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}

async fn pair() -> Pair {
    bounded(pair_inner()).await
}

async fn pair_inner() -> Pair {
    pair_with_data(b"a file that must not pin the actor".to_vec()).await
}

async fn pair_with_data(data: Vec<u8>) -> Pair {
    pair_with_service(data, true).await
}

/// Build real encrypted test data before starting the async-stall watchdog. A full-size
/// chunk is 8 MiB: its bounded local cryptographic work is not a five-second performance
/// contract. Membership, subscription and catch-up then share ONE five-second deadline.
/// `serve_join = false` injects an unavailable peer to prove that deadline still rejects.
async fn pair_with_service(data: Vec<u8>, serve_join: bool) -> Pair {
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let clock_hook: ClockHook = Arc::new(std::sync::Mutex::new(None));
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
    let cid = holder
        .add_file("test.bin", "application/octet-stream", "", &data)
        .await
        .unwrap();
    let invite = holder.mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let joiner = MlsDevice::generate().unwrap();
    // The concrete MemNetwork subscriptions/publication above are local operations. Any
    // future peer wait belongs BELOW this watchdog, which must not restart between stages.
    let setup_clock = catcoms_rt::SystemClock;
    let setup_deadline = setup_clock.sleep(Duration::from_secs(5));
    tokio::pin!(setup_deadline);
    let mut requester = {
        let joining = Server::join(
            GatedTransport {
                inner: hub.join(PeerId::from_u64(2)),
                gate: gate.clone(),
                entered,
                gated_peer: std::sync::Mutex::new(None),
                connection_events: std::sync::Mutex::new(VecDeque::new()),
                legacy_blob_only: std::sync::atomic::AtomicBool::new(false),
                sent_blob_requests: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            },
            joiner,
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ValidationClock {
                inner: clock.clone(),
                hook: clock_hook.clone(),
            }),
            "bob",
            holder_peer,
            &invite,
        );
        tokio::pin!(joining);
        loop {
            tokio::select! {
                biased;
                _ = &mut setup_deadline => panic!("file-transfer pair stalled during member join"),
                result = &mut joining => break result.unwrap(),
                _ = holder.sync_once(), if serve_join => {}
            }
        }
    };
    tokio::select! {
        biased;
        _ = &mut setup_deadline => panic!("file-transfer pair stalled during file subscription"),
        result = requester.open_files() => result.unwrap(),
    }
    {
        let catchup = requester.request_files_catchup(holder_peer);
        tokio::pin!(catchup);
        loop {
            tokio::select! {
                biased;
                _ = &mut setup_deadline => panic!("file-transfer pair stalled during file catch-up"),
                result = &mut catchup => { result.unwrap(); break; },
                _ = holder.sync_once() => {}
            }
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
        clock_hook,
    }
}

#[tokio::test]
#[should_panic(expected = "file-transfer pair stalled during member join")]
async fn pair_network_watchdog_rejects_an_unserved_join() {
    pair_with_service(b"unserved join".to_vec(), false).await;
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
    let mut transfers = isolated_transfers();
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
    let mut transfers = isolated_transfers();
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
    let mut transfers = isolated_transfers();
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
    let mut transfers = isolated_transfers();
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
    let mut transfers = isolated_transfers();
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
    assert!(
        requester.sync.keep_auto_source_available(&cid),
        "real caught-up current provider is eligible"
    );
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
async fn accepted_crossing_range_chunk_renews_quiet_deadline_before_fetching_the_next_chunk() {
    // Keep the observer bound unchanged, but identify the exact stalled owner. A generic
    // timeout here cannot distinguish fixture startup from a missed transfer completion.
    async fn stage<F: Future>(name: &'static str, future: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(5), future)
            .await
            .unwrap_or_else(|error| panic!("crossing-range stage {name} stalled: {error}"))
    }
    let data: Vec<_> = (0..CHUNK_BYTES + 64).map(|i| (i % 223) as u8).collect();
    // The helper bounds actual network waits after fixed 8 MiB cryptographic preparation;
    // it does not treat aggregate fixture construction as a five-second performance test.
    let Pair {
        mut requester,
        holder,
        cid,
        mut entered,
        gate,
        clock,
        ..
    } = pair_with_data(data.clone()).await;
    let manifest = FileManifest::decode_or_legacy(&requester.files()[0].file_ref).unwrap();
    assert_eq!(manifest.chunks.len(), 2);
    let first = manifest.chunks[0].ciphertext_cid;
    requester
        .sync
        .put_blob(&holder.sync.get_blob(&first).unwrap())
        .unwrap();
    let version = requester.file_head(&cid).unwrap().manifest_version;
    let (source, _events, source_task) = spawn(holder);
    // This FIFO command proves the actual actor finished setup before the shared clock
    // jumps. Starting the source is a prerequisite, not part of the renewal being tested.
    assert_eq!(stage("source startup", source.member_count()).await, 2);
    let mut transfers = isolated_transfers();
    let old_deadline = clock.monotonic_ms() + READ_MS;
    let at_accept = clock.clone();
    transfers.before_storage_admission = Some(Box::new(move || {
        at_accept.set_ms(old_deadline - 1);
    }));
    let start = CHUNK_BYTES - 16;
    let (reply, result) = oneshot::channel();
    transfers.range_cancellable(
        &mut requester,
        cid.as_bytes().to_vec(),
        version,
        start as u64,
        32,
        None,
        reply,
    );
    assert_eq!(clock.monotonic_ms(), old_deadline - 1);
    let attempt = stage("second chunk transport admission", entered.recv())
        .await
        .unwrap();
    clock.set_ms(old_deadline + 1);
    assert!(!attempt.is_cancelled(), "renewed request remains current");
    gate.add_permits(1);
    eprintln!(
        "crossing-range completion: mono={}, requests={}, running={}, inflight={}",
        clock.monotonic_ms(),
        requester
            .sync
            .transport()
            .sent_blob_requests
            .load(std::sync::atomic::Ordering::Acquire),
        transfers.running.len(),
        transfers.inflight.len(),
    );
    let completed = tokio::time::timeout(Duration::from_secs(5), transfers.next())
        .await
        .unwrap_or_else(|error| {
            panic!(
                "crossing-range second chunk completion stalled: mono={}, requests={}, running={}, inflight={}: {error}",
                clock.monotonic_ms(),
                requester
                    .sync
                    .transport()
                    .sent_blob_requests
                    .load(std::sync::atomic::Ordering::Acquire),
                transfers.running.len(),
                transfers.inflight.len(),
            )
        })
        .unwrap();
    let TransferStep::Network(network) = &completed else {
        panic!("crossing range must complete its actual second-chunk request")
    };
    assert_eq!(network.job.index, 1);
    assert_eq!(network.job.deadline, old_deadline - 1 + READ_MS);
    assert!(
        network.result.is_ok(),
        "second-chunk worker did not complete normally: {:?}",
        network.result.as_ref().err()
    );
    transfers.complete(&mut requester, completed);
    assert!(
        transfers.is_empty(),
        "crossing range must settle its result"
    );
    assert_eq!(
        stage("range result", result).await.unwrap().unwrap().bytes,
        data[start..start + 32]
    );
    drop(attempt);
    stage("source shutdown", async {
        source.shutdown().await;
        source_task.await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn kept_request_windows_charge_the_actual_tail_and_legacy_frame() {
    let Pair { requester, cid, .. } = pair().await;
    let manifest = FileManifest::decode_or_legacy(&requester.files()[0].file_ref).unwrap();
    assert_eq!(manifest.plaintext_cid, cid);
    let reference = &manifest.chunks[0];
    let bound = kept_ciphertext_bound(reference).unwrap();
    assert_eq!(
        kept_request_window(reference, true, 65_536, bound - 3).unwrap(),
        (3, 3)
    );
    assert!(kept_request_window(reference, true, 65_536, bound).is_err());
    assert!(kept_request_window(reference, true, 65_536, bound + 1).is_err());
    assert!(kept_request_window(reference, true, 0, 0).is_err());
    assert_eq!(
        kept_request_window(reference, false, 1, 0).unwrap(),
        (bound, catcoms_rt::MAX_REQUEST_RESPONSE_FRAME_BYTES as u64)
    );
    let mut oversized = reference.clone();
    oversized.size = CHUNK_BYTES as u64 + 1;
    assert!(kept_request_window(&oversized, true, 1, 0).is_err());
}

#[tokio::test]
async fn kept_legacy_provider_fallback_spends_the_whole_frame_and_completes() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        mut entered,
        ..
    } = pair().await;
    requester
        .sync
        .transport()
        .legacy_blob_only
        .store(true, std::sync::atomic::Ordering::Release);
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    gate.add_permits(2);
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    let initial_bytes = requester.sync.kept_files().transfers[0].remaining.bytes;
    let local = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, local);
    let _page = bounded(entered.recv()).await.unwrap();
    let unsupported = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, unsupported);
    let _whole = bounded(entered.recv()).await.unwrap();
    let reply = bounded(transfers.next()).await.unwrap();
    let TransferStep::Network(attempt) = &reply else {
        panic!("expected actual whole-blob exchange")
    };
    assert!(!attempt.paged);
    assert!(
        initial_bytes - requester.sync.kept_files().transfers[0].remaining.bytes
            >= catcoms_rt::MAX_REQUEST_RESPONSE_FRAME_BYTES as u64
    );
    transfers.complete(&mut requester, reply);
    bounded(result).await.unwrap().unwrap();
    assert!(requester.sync.kept_files().files[0].checked);
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn kept_request_expired_before_first_poll_never_enters_transport() {
    let Pair {
        mut requester,
        holder: _holder,
        cid,
        mut entered,
        clock,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    let local = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, local);
    // This test's single-thread executor has not polled the spawned network worker. Advance
    // beyond its original authority before yielding, rather than sleeping for scheduler order.
    clock.advance_ms(catcoms_storage::kept::KEEP_EXECUTION_SLICE_MS + 1);
    let expired = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, expired);
    assert!(bounded(result).await.unwrap().is_err());
    assert!(
        entered.try_recv().is_err(),
        "expired work must not invoke the transport"
    );
    assert!(!requester.sync.kept_files().transfers[0].running);
}

#[tokio::test]
async fn kept_queued_request_loses_authority_on_deadline_or_same_cid_replacement() {
    for replace in [false, true] {
        let Pair {
            mut requester,
            holder: _holder,
            cid,
            mut entered,
            clock,
            gate,
            ..
        } = pair().await;
        let dir = tempfile::tempdir().unwrap();
        attach_kept(&mut requester, dir.path());
        let mut transfers = isolated_transfers();
        let (reply, result) = oneshot::channel();
        transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
        let local = bounded(transfers.next()).await.unwrap();
        let TransferStep::Local(job) = &local else {
            panic!("expected local continuation")
        };
        let old_run = job.keep.as_ref().unwrap().token;
        transfers.complete(&mut requester, local);
        let queued = bounded(entered.recv()).await.unwrap();
        assert!(!queued.is_cancelled());
        let new_run = if replace {
            old_run.pause(&mut requester).unwrap();
            let plan = requester.sync.kept_files().transfers[0].plan.clone();
            Some(requester.sync.begin_keep(plan).unwrap())
        } else {
            clock.advance_ms(catcoms_storage::kept::KEEP_EXECUTION_SLICE_MS + 1);
            None
        };
        // This is checked before the upper task is polled or can publish its cancel bit.
        assert!(
            queued.is_cancelled(),
            "the lower queue sees immutable native authority"
        );
        gate.add_permits(1);
        let refused = bounded(transfers.next()).await.unwrap();
        transfers.complete(&mut requester, refused);
        assert!(bounded(result).await.unwrap().is_err());
        assert_eq!(
            requester
                .sync
                .transport()
                .sent_blob_requests
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        if let Some(run) = new_run {
            assert!(
                requester.sync.kept_files().transfers[0].running,
                "old completion cannot pause the replacement run"
            );
            requester.sync.pause_keep(run).unwrap();
        }
    }
}

#[tokio::test]
async fn kept_signed_oversized_chunk_is_rejected_before_staging_or_progress() {
    let Pair {
        mut requester,
        mut holder,
        cid,
        gate,
        mut entered,
        ..
    } = pair().await;
    // Retain genuine ciphertext and provider authentication, but publish an internally
    // consistent manifest whose claimed plaintext size is smaller than that ciphertext.
    // Authentication proves who answered; it must not bypass the retained byte reservation.
    let data = vec![9; 65_536];
    let oversized_cid = holder
        .add_file("oversized.bin", "application/octet-stream", "", &data)
        .await
        .unwrap();
    let original = FileManifest::decode_or_legacy(
        &holder
            .files()
            .iter()
            .find(|entry| {
                FileManifest::decode_or_legacy(&entry.file_ref)
                    .is_ok_and(|m| m.plaintext_cid == oversized_cid)
            })
            .unwrap()
            .file_ref,
    )
    .unwrap();
    let mut claimed = original;
    claimed.total_size = 1;
    claimed.chunks[0].size = 1;
    requester
        .sync
        .post(DocType::FileIndex, 0, |doc| {
            crate::write_file_entry(
                doc,
                "claimed.bin",
                "application/octet-stream",
                "",
                &claimed.encode(),
                crate::FileExpiry::Never,
                None,
            )
        })
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    gate.add_permits(1);
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(
        &mut requester,
        oversized_cid.as_bytes().to_vec(),
        None,
        reply,
    );
    // First continuation prepares the request; the second delivers the real signed page.
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    let _lower = bounded(entered.recv()).await.unwrap();
    let step = bounded(transfers.next()).await.unwrap();
    let TransferStep::Network(attempt) = &step else {
        panic!("expected signed page response")
    };
    assert!(attempt.paged);
    assert!(attempt.job.staged.is_empty());
    transfers.complete(&mut requester, step);
    assert!(bounded(result)
        .await
        .unwrap()
        .unwrap_err()
        .contains("declared size"));
    let paused = requester.sync.kept_files();
    assert_eq!(paused.transfers[0].verified_chunks, 0);
    assert!(!paused.transfers[0].running);
    assert!(paused.files.is_empty());
    assert!(requester.sync.blob_cids().is_empty());
    assert!(!requester.file_available(&oversized_cid));
    assert_ne!(cid, oversized_cid);
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn cancelled_or_timed_out_requests_hold_capacity_until_transport_retirement() {
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
    let (source, _events, source_task) = spawn(holder);
    // A per-test process pool avoids cross-test timing changing this capacity assertion.
    let mut transfers = FileTransfers {
        lifetime: watch::channel(true).0,
        running: JoinSet::new(),
        jobs: Arc::new(Semaphore::new(SERVER_ATTEMPTS)),
        process_jobs: Arc::new(Semaphore::new(PROCESS_ATTEMPTS)),
        slots: Arc::new(Semaphore::new(1)),
        process: Arc::new(Semaphore::new(1)),
        inflight: HashSet::new(),
        automatic_due: None,
        automatic_after: None,
        before_storage_admission: None,
    };
    let (reply, mut result) = oneshot::channel();
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
    // Transient local pressure must keep the admitted read alive. It cannot borrow/refund the
    // old request's permit, spend another provider attempt, or replace the selected candidate.
    let resumed = resume_paused(&mut transfers, &clock, ROUND_BACKOFF_MS).await;
    transfers.complete(&mut requester, resumed);
    assert!(matches!(
        result.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(
        transfers.running.len(),
        1,
        "one bounded parked job, no busy loop"
    );
    assert_eq!(
        transfers.slots.available_permits(),
        0,
        "a refused retry must not refund the live request's permit"
    );
    drop(attempt);
    assert_eq!(transfers.slots.available_permits(), 1);
    gate.add_permits(1);
    let resumed = resume_paused(&mut transfers, &clock, BUSY_PEER_BACKOFF_MS).await;
    transfers.complete(&mut requester, resumed);
    drop(bounded(entered.recv()).await.unwrap());
    let completed = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, completed);
    assert_eq!(bounded(result).await.unwrap().unwrap().0, data);
    assert_eq!(transfers.slots.available_permits(), 1);
    source.shutdown().await;
    bounded(source_task).await.unwrap();
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
        [8; 32],
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
    let mut transfers = isolated_transfers();
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
async fn cancelled_kept_wait_preserves_reservation_without_storing_late_bytes() {
    let Pair {
        mut requester,
        holder: _holder,
        cid,
        mut entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let mut transfers = isolated_transfers();
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
    let paused = requester.sync.kept_files();
    assert!(paused.allocated_bytes > 4096);
    assert_eq!(paused.transfers.len(), 1);
    assert_eq!(paused.transfers[0].verified_chunks, 0);
    assert!(!paused.transfers[0].running);
    assert!(requester.sync.blob_cids().is_empty());
    // Dropping a waiter pauses durable work. Only explicit local release cancels ownership;
    // its permanent format marker remains, so an older reader cannot reopen the new format.
    requester.sync.forget_kept(&cid).unwrap();
    let released = requester.sync.kept_files();
    assert!(released.transfers.is_empty());
    assert_eq!(released.allocated_bytes, 4096);
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
    let mut transfers = isolated_transfers();
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
    let paused = holder.sync.kept_files();
    assert!(paused.files.is_empty());
    assert_eq!(paused.transfers[0].verified_chunks, 1);
    assert!(!paused.transfers[0].running);
    let reserved = paused.allocated_bytes;
    // The resumed job must rebuild the whole-file hash from its private durable prefix. It
    // cannot depend on the cache retaining that prefix or count a single saved part as a file.
    let manifest = FileManifest::decode_or_legacy(&paused.transfers[0].plan.manifest).unwrap();
    holder
        .sync
        .delete_blob(&manifest.chunks[0].ciphertext_cid)
        .unwrap();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    for _ in 0..2 {
        let step = bounded(transfers.next()).await.unwrap();
        transfers.complete(&mut holder, step);
    }
    bounded(result).await.unwrap().unwrap();
    let completed = holder.sync.kept_files();
    assert_eq!(completed.allocated_bytes, reserved);
    assert!(completed.files[0].checked);
    assert_eq!(completed.transfers[0].verified_chunks, 2);
}

#[tokio::test]
async fn queued_commands_outrank_even_ready_local_keep_continuations() {
    let Pair {
        mut holder, cid, ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut holder, dir.path());
    let scope = catcoms_storage::kept::KeepScope {
        group: holder.sync.group_id(),
        device: holder.sync.device_id(),
    };
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
    // Shutdown leaves a paused, non-serving reservation. Binding the next exclusive mount to
    // the same full identity recovers that ownership without starting or replenishing work.
    let mut reopened = catcoms_storage::kept::KeptBlobStore::open(
        Box::new(catcoms_storage::MemoryBlobStore::new()),
        dir.path().join("kept"),
        [6; 32],
        [8; 32],
        ChaCha20Rng::seed_from_u64(92),
    );
    catcoms_storage::BlobStore::configure_keep_scope(&mut reopened, scope).unwrap();
    let recovered = catcoms_storage::BlobStore::kept_files(&reopened);
    assert!(recovered.files.is_empty());
    assert!(recovered.allocated_bytes > 4096);
    assert_eq!(recovered.transfers.len(), 1);
    assert_eq!(recovered.transfers[0].verified_chunks, 0);
    assert!(!recovered.transfers[0].running);
    assert_eq!(recovered.transfers[0].grant, 1);
}

/// Create the intent through the real application admission, then lose only its UI waiter.
/// No worker has read a prefix or spent a durable execution slice yet.
async fn interrupt_keep_before_first_step<T: MeshTransport + 'static>(
    server: &mut Server<T, ChaCha20Rng>,
    cid: Cid,
) {
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(server, cid.as_bytes().to_vec(), None, reply);
    drop(result);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(server, step);
    assert!(!server.sync.kept_files().transfers[0].running);
}

#[tokio::test]
async fn automatic_keep_resumes_reopened_intent_on_owned_actor_wake_without_ui() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        clock,
        entered: _entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    interrupt_keep_before_first_step(&mut requester, cid).await;
    let before = requester.sync.kept_files().transfers[0].clone();
    requester.set_blob_store(Box::new(catcoms_storage::MemoryBlobStore::new()));
    attach_kept(&mut requester, dir.path());
    assert_eq!(
        requester.sync.kept_files().transfers[0].remaining,
        before.remaining
    );
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, _events, task) = spawn(requester);
    let (session, receiver) = watch::channel(false);
    actor.set_keep_session(Some(RequestCancellation::new(receiver, None)));
    assert!(bounded(actor.kept_files()).await.files.is_empty());
    gate.add_permits(8);
    clock.advance_ms(AUTOMATIC_KEEP_MS);
    bounded(async {
        loop {
            let view = actor.kept_files().await;
            if view
                .files
                .iter()
                .any(|file| file.plan.cid == cid && file.checked)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    actor.shutdown().await;
    bounded(task).await.unwrap();
    source.shutdown().await;
    bounded(source_task).await.unwrap();
    drop(session);
}

#[tokio::test]
async fn automatic_keep_native_session_revokes_before_local_start_and_actual_lower_admission() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        mut entered,
        clock,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    interrupt_keep_before_first_step(&mut requester, cid).await;
    let (source, _events, source_task) = spawn(holder);
    let mut transfers = isolated_transfers();
    assert!(
        requester.sync.keep_auto_source_available(&cid),
        "real caught-up current provider is eligible"
    );
    let (old_owner, old_receiver) = watch::channel(false);
    let old = RequestCancellation::new(old_receiver, None);
    assert!(transfers.pump_automatic(&mut requester, None).is_none());
    let original = requester.sync.kept_files().transfers[0].remaining;
    assert_eq!(
        transfers.pump_automatic(&mut requester, Some(old.clone())),
        Some(clock.monotonic_ms() + AUTOMATIC_KEEP_MS)
    );
    clock.advance_ms(AUTOMATIC_KEEP_MS);
    transfers.pump_automatic(&mut requester, Some(old.clone()));
    old_owner.send_replace(true);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    assert_eq!(requester.sync.kept_files().transfers[0].remaining, original);
    assert!(entered.try_recv().is_err());
    let (fresh_owner, receiver) = watch::channel(false);
    let fresh = RequestCancellation::new(receiver, None);
    transfers.pump_automatic(&mut requester, Some(fresh.clone()));
    clock.advance_ms(AUTOMATIC_KEEP_MS);
    transfers.pump_automatic(&mut requester, Some(fresh.clone()));
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    let lower = bounded(entered.recv()).await.unwrap();
    assert!(!lower.is_cancelled());
    fresh_owner.send_replace(true);
    assert!(
        lower.is_cancelled(),
        "queued actual lower work sees original native retirement synchronously"
    );
    let charged = requester.sync.kept_files().transfers[0].remaining;
    gate.add_permits(1);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    assert_eq!(
        requester
            .sync
            .transport()
            .sent_blob_requests
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert_eq!(requester.sync.kept_files().transfers[0].remaining, charged);
    assert!(!requester.sync.kept_files().transfers[0].running);
    assert!(transfers
        .pump_automatic(&mut requester, Some(old))
        .is_none());
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn automatic_keep_providerless_scan_and_paused_intent_do_not_spend_or_rearm_each_turn() {
    let Pair {
        mut requester,
        holder,
        cid,
        clock,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    interrupt_keep_before_first_step(&mut requester, cid).await;
    drop(holder); // Hub connection snapshot now has no eligible source.
    let (owner, receiver) = watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let mut transfers = isolated_transfers();
    let before = requester.sync.kept_files().transfers[0].clone();
    let due = transfers
        .pump_automatic(&mut requester, Some(session.clone()))
        .unwrap();
    clock.advance_ms(AUTOMATIC_KEEP_MS - 1);
    assert_eq!(
        transfers.pump_automatic(&mut requester, Some(session.clone())),
        Some(due)
    );
    clock.advance_ms(1);
    assert_eq!(
        transfers.pump_automatic(&mut requester, Some(session.clone())),
        Some(due + AUTOMATIC_KEEP_MS)
    );
    assert!(transfers.is_empty());
    assert_eq!(
        requester.sync.kept_files().transfers[0].remaining,
        before.remaining
    );
    requester
        .sync
        .set_keep_paused(&cid, before.transfer, before.intent, true)
        .unwrap();
    assert!(transfers
        .pump_automatic(&mut requester, Some(session))
        .is_none());
    assert!(transfers.is_empty());
    drop(owner);
}

#[tokio::test]
async fn stopping_completed_repair_retires_lower_admission_without_releasing_saved_copy() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        mut entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    gate.add_permits(8);
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    while let Some(step) = transfers.next().await {
        transfers.complete(&mut requester, step);
    }
    bounded(result).await.unwrap().unwrap();
    let view = requester.sync.kept_files();
    let chunk = view.files[0].plan.chunks[0].0;
    let allocated = view.allocated_bytes;
    std::fs::remove_file(
        dir.path()
            .join("kept")
            .join("transfers-v2")
            .join(cid.to_hex())
            .join(chunk.to_hex()),
    )
    .unwrap();
    gate.forget_permits(gate.available_permits());
    while entered.try_recv().is_ok() {}
    let before_requests = requester
        .sync
        .transport()
        .sent_blob_requests
        .load(std::sync::atomic::Ordering::Acquire);
    let (stop, receiver) = watch::channel(false);
    let (reply, result) = oneshot::channel();
    transfers.keep(
        &mut requester,
        cid.as_bytes().to_vec(),
        Some(RequestCancellation::new(receiver, None)),
        reply,
    );
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    let lower = bounded(entered.recv()).await.unwrap();
    stop.send_replace(true);
    assert!(lower.is_cancelled());
    gate.add_permits(1);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    assert!(bounded(result).await.unwrap().is_err());
    let after = requester.sync.kept_files();
    assert_eq!(after.allocated_bytes, allocated);
    assert_eq!(after.files.len(), 1);
    assert!(!after.files[0].checked);
    assert_eq!(
        after.transfers[0].phase,
        catcoms_storage::kept::KeepPhase::Completed
    );
    assert!(!after.transfers[0].running);
    assert_eq!(
        requester
            .sync
            .transport()
            .sent_blob_requests
            .load(std::sync::atomic::Ordering::Acquire),
        before_requests
    );
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn keyless_actor_refuses_keep_before_file_lookup_reservation_or_worker_admission() {
    let hub = Hub::new();
    let device = MlsDevice::generate().unwrap();
    let group = catcoms_mls::ServerGroup::create(&device).unwrap();
    let mut keyless = catcoms_sync::ChannelSync::new_joined(
        hub.join(PeerId::from_u64(991)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(991),
        Box::new(ManualClock::new(1_000)),
        catcoms_sync::RoutingState::default(),
    );
    let snapshot = keyless.snapshot().unwrap();
    drop(keyless);
    let mut server = Server::restore(
        &snapshot,
        hub.join(PeerId::from_u64(991)),
        ChaCha20Rng::seed_from_u64(992),
        Box::new(ManualClock::new(1_000)),
        "keyless member",
    )
    .unwrap();
    assert!(!server.sync.has_file_key());
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut server, dir.path());
    let before = server.sync.kept_files();
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(
        &mut server,
        Cid::of(b"requested file").as_bytes().to_vec(),
        None,
        reply,
    );
    assert_eq!(
        bounded(result).await.unwrap().unwrap_err(),
        catcoms_sync::SyncError::FileKeyUnavailable.to_string()
    );
    assert!(transfers.is_empty());
    assert_eq!(server.sync.kept_files(), before);
    drop(server);
    drop(dir);
}

#[tokio::test]
async fn kept_bad_first_wrapped_key_falls_back_to_saved_whole_candidate() {
    let Pair {
        mut holder, cid, ..
    } = pair().await;
    let good = add_bad_wrapped_variant(&mut holder, &cid).await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut holder, dir.path());
    let mut transfers = isolated_transfers();
    let (reply, mut result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    let admitted = holder.sync.kept_files().transfers[0].clone();
    let admitted_allocation = holder.sync.kept_files().allocated_bytes;
    assert_ne!(
        admitted.plan.version,
        crate::file_manifest_version(&good.encode())
    );
    for _ in 0..8 {
        if let Ok(outcome) = result.try_recv() {
            outcome.expect("bad first key must not strand a healthy saved candidate");
            break;
        }
        let step = bounded(transfers.next())
            .await
            .expect("a candidate remains");
        transfers.complete(&mut holder, step);
    }
    let saved = holder.sync.kept_files();
    assert_eq!(saved.files.len(), 1, "complete one whole candidate");
    assert!(saved.files[0].checked);
    assert_eq!(
        saved.files[0].plan.version,
        crate::file_manifest_version(&good.encode())
    );
    assert_eq!(saved.transfers[0].transfer, admitted.transfer);
    assert_eq!(saved.transfers[0].grant, admitted.grant);
    assert_eq!(saved.transfers[0].intent, admitted.intent);
    assert_eq!(saved.allocated_bytes, admitted_allocation);
}

#[tokio::test]
async fn kept_midfile_candidate_fallback_restarts_whole_hash_and_preserves_saved_candidates_after_unlisting(
) {
    let Pair {
        mut holder,
        hub,
        clock,
        ..
    } = pair().await;
    let bytes = vec![3; CHUNK_BYTES + 1];
    let cid = holder
        .add_file("two-candidates.bin", "application/octet-stream", "", &bytes)
        .await
        .unwrap();
    let good = FileManifest::decode_or_legacy(
        &holder
            .files()
            .iter()
            .find(|entry| entry.cid == cid.as_bytes())
            .unwrap()
            .file_ref,
    )
    .unwrap();
    let good_version = crate::file_manifest_version(&good.encode());
    let mut bad = good.clone();
    // A separate valid first ciphertext makes candidate-prefix splicing observable. The second
    // reference has a bad exact key; its failure must restart hashing at the other first chunk.
    let (first, ciphertext) = holder
        .sync
        .seal_file(&bytes[..CHUNK_BYTES], "application/octet-stream")
        .unwrap();
    holder.sync.put_blob(&ciphertext).unwrap();
    bad.chunks[0] = first;
    for nonce in 1u16..=u16::MAX {
        bad.chunks[1].wrapped_key.nonce[..2].copy_from_slice(&nonce.to_be_bytes());
        if crate::file_manifest_version(&bad.encode()) < good_version {
            break;
        }
    }
    assert!(crate::file_manifest_version(&bad.encode()) < good_version);
    holder
        .sync
        .post(DocType::FileIndex, 0, |doc| {
            crate::write_file_entry(
                doc,
                "bad-two.bin",
                "legacy",
                "",
                &bad.encode(),
                crate::FileExpiry::Never,
                None,
            )
        })
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut holder, dir.path());
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    let admitted = holder
        .sync
        .kept_files()
        .transfers
        .iter()
        .find(|t| t.plan.cid == cid)
        .unwrap()
        .clone();
    let first = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut holder, first);
    assert_eq!(holder.sync.kept_files().transfers[0].verified_chunks, 1);
    holder
        .sync
        .post(DocType::FileIndex, crate::FILE_INDEX_DOC, |doc| {
            crate::delete_file_entry(doc, cid.as_bytes(), None)
        })
        .await
        .unwrap();
    assert!(holder.file_head(&cid).is_none());
    // Persist the whole-candidate switch, close the waiter, then reopen the actual server
    // snapshot and disk store. The shared listing cannot reconstruct any candidate now.
    let failed_second = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut holder, failed_second);
    assert_eq!(
        holder.sync.kept_files().transfers[0].plan.version,
        good_version
    );
    drop(result);
    let cancelled = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut holder, cancelled);
    let suspended = holder.sync.kept_files();
    assert!(!suspended.transfers[0].running);
    assert!(!suspended.transfers[0].blocked);
    let snapshot = holder.snapshot().unwrap();
    let peer = holder.sync.local_peer();
    drop(transfers);
    drop(holder);
    let mut holder = Server::restore(
        &snapshot,
        hub.join(peer),
        ChaCha20Rng::seed_from_u64(99),
        Box::new(clock),
        "alice",
    )
    .unwrap();
    attach_kept(&mut holder, dir.path());
    assert_eq!(holder.sync.kept_files(), suspended);
    assert!(holder.file_head(&cid).is_none());
    let mut transfers = isolated_transfers();
    let (reply, mut result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    for _ in 0..8 {
        if let Ok(outcome) = result.try_recv() {
            outcome.expect("mid-file fallback must hash the complete saved alternative");
            break;
        }
        let step = bounded(transfers.next())
            .await
            .expect("saved alternative continuation");
        transfers.complete(&mut holder, step);
    }
    let saved = holder.sync.kept_files();
    assert_eq!(saved.files.len(), 1);
    assert!(saved.files[0].checked);
    assert_eq!(saved.files[0].plan.version, good_version);
    assert_eq!(saved.transfers[0].transfer, admitted.transfer);
    assert_eq!(saved.transfers[0].grant, admitted.grant);
    assert!(saved.transfers[0].remaining.slices < admitted.remaining.slices);
    assert!(
        holder.file_head(&cid).is_none(),
        "saved fallback cannot republish an unlisted entry"
    );
}

#[tokio::test]
async fn kept_network_quiet_deadline_rotates_to_a_saved_local_candidate_without_excluding_the_source(
) {
    let Pair {
        mut requester,
        holder: _holder,
        cid,
        data,
        clock,
        mut entered,
        ..
    } = pair().await;
    let first = FileManifest::decode_or_legacy(&requester.files()[0].file_ref).unwrap();
    let first_version = crate::file_manifest_version(&first.encode());
    let (other, ciphertext) = (0..256)
        .find_map(|_| {
            let (reference, bytes) = requester
                .sync
                .seal_file(&data, "application/octet-stream")
                .unwrap();
            let mut manifest = first.clone();
            manifest.chunks[0] = reference;
            (crate::file_manifest_version(&manifest.encode()) > first_version)
                .then_some((manifest, bytes))
        })
        .expect("deterministic RNG supplies a sorted later healthy candidate");
    requester
        .sync
        .post(DocType::FileIndex, 0, |doc| {
            crate::write_file_entry(
                doc,
                "later.bin",
                "legacy",
                "",
                &other.encode(),
                crate::FileExpiry::Never,
                None,
            )
        })
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let mut transfers = isolated_transfers();
    let (reply, mut result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    let admitted = requester.sync.kept_files().transfers[0].clone();
    assert_eq!(admitted.plan.version, first_version);
    // Dispatch just before the ORIGINAL quiet deadline. The newly prepaid 15s work slice is
    // still live when that earlier per-chunk deadline is observed by network completion.
    clock.advance_ms(READ_MS - 100);
    let local = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, local);
    let old_lower = bounded(entered.recv()).await.unwrap();
    requester.sync.put_blob(&ciphertext).unwrap(); // Another ordinary read acquired saved candidate B.
    clock.advance_ms(100);
    let network = bounded(transfers.next()).await.unwrap();
    let TransferStep::Network(attempt) = &network else {
        panic!("expected expired source attempt");
    };
    assert!(requester
        .sync
        .validate_keep_work(attempt.keep_work.as_ref().unwrap())
        .is_ok());
    assert_eq!(attempt.job.deadline, clock.monotonic_ms());
    transfers.complete(&mut requester, network);
    for _ in 0..6 {
        if let Ok(outcome) = result.try_recv() {
            outcome.expect("quiet timeout must permit healthy saved fallback");
            break;
        }
        let step = bounded(transfers.next())
            .await
            .expect("saved local candidate continuation");
        transfers.complete(&mut requester, step);
    }
    let saved = requester.sync.kept_files();
    assert_eq!(saved.files.len(), 1);
    assert!(saved.files[0].checked);
    assert_eq!(
        saved.files[0].plan.version,
        crate::file_manifest_version(&other.encode())
    );
    assert_eq!(saved.transfers[0].transfer, admitted.transfer);
    assert_eq!(saved.transfers[0].grant, admitted.grant);
    assert_eq!(saved.transfers[0].intent, admitted.intent);
    assert!(old_lower.is_cancelled());
    // Timeout is not a deterministic manifest verdict: the original candidate remains saved
    // and selectable for a later explicit check, with the completed B winner still owned.
    let run = requester
        .sync
        .begin_keep(saved.transfers[0].plan.clone())
        .unwrap();
    let (run, plan) = requester
        .sync
        .fallback_keep(
            run,
            &saved.transfers[0].plan.version,
            &[saved.transfers[0].plan.version],
            false,
        )
        .unwrap()
        .expect("source A was not marked bad");
    assert_eq!(plan.version, first_version);
    requester.sync.pause_keep(run).unwrap();
}

#[tokio::test]
async fn kept_resume_validates_all_saved_candidates_before_run_admission_or_spend() {
    let Pair {
        mut holder, cid, ..
    } = pair().await;
    let encoded = holder.files()[0].file_ref.clone();
    let (good, manifest) = kept_plan(encoded, cid).unwrap();
    for fault in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        attach_kept(&mut holder, dir.path());
        let mut other = manifest.clone();
        other.chunks[0].plaintext_cid = Cid::of(b"incompatible plaintext chunk");
        let (mut other, _) = kept_plan(other.encode(), cid).unwrap();
        match fault {
            0 => {} // Each plan decodes, but their plaintext chunks are incompatible.
            1 => other.version = [0; 32],
            2 => other.chunks[0].1 += 1,
            _ => {}
        }
        let mut plans = vec![good.clone(), other];
        plans.sort_by_key(|p| p.version);
        let selected = plans.iter().position(|p| p == &good).unwrap();
        let listing = if fault == 3 {
            [0; 32]
        } else {
            file_resolution::manifest_set_version(plans.iter().map(|p| p.version))
        };
        let run = holder
            .sync
            .begin_keep_variants(plans, selected, listing)
            .unwrap();
        holder.sync.pause_keep(run).unwrap();
        let before = holder.sync.kept_files();
        let mut transfers = isolated_transfers();
        let (reply, result) = oneshot::channel();
        transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
        assert!(bounded(result).await.unwrap().is_err());
        assert!(transfers.is_empty());
        assert_eq!(holder.sync.kept_files(), before);
        holder.set_blob_store(Box::new(catcoms_storage::MemoryBlobStore::new()));
    }
}

#[tokio::test]
async fn kept_signed_bad_key_response_falls_back_without_caching_failed_ciphertext_or_refunding_work(
) {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        entered: _entered,
        ..
    } = pair().await;
    let good = add_bad_wrapped_variant(&mut requester, &cid).await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    gate.add_permits(4);
    let mut transfers = isolated_transfers();
    let (reply, mut result) = oneshot::channel();
    transfers.keep(&mut requester, cid.as_bytes().to_vec(), None, reply);
    let admitted = requester.sync.kept_files();
    let first = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, first);
    let response = bounded(transfers.next()).await.unwrap();
    let TransferStep::Network(attempt) = &response else {
        panic!("expected authenticated source response");
    };
    assert_ne!(attempt.reference.encode(), good.chunks[0].encode());
    assert!(attempt.result.is_ok());
    transfers.complete(&mut requester, response);
    assert!(
        !requester.sync.has_blob(&good.chunks[0].ciphertext_cid),
        "failed exact decryption never entered ordinary cache or completed retention"
    );
    for _ in 0..8 {
        if let Ok(outcome) = result.try_recv() {
            outcome.expect("healthy exact reference to the same signed ciphertext remains usable");
            break;
        }
        let step = bounded(transfers.next())
            .await
            .expect("whole fallback continuation");
        transfers.complete(&mut requester, step);
    }
    let saved = requester.sync.kept_files();
    assert_eq!(saved.files.len(), 1);
    assert!(saved.files[0].checked);
    assert_eq!(
        saved.files[0].plan.version,
        crate::file_manifest_version(&good.encode())
    );
    assert_eq!(saved.allocated_bytes, admitted.allocated_bytes);
    assert_eq!(saved.transfers[0].transfer, admitted.transfers[0].transfer);
    assert_eq!(saved.transfers[0].grant, admitted.transfers[0].grant);
    assert_eq!(saved.transfers[0].intent, admitted.transfers[0].intent);
    assert_eq!(
        admitted.transfers[0].remaining.requests - saved.transfers[0].remaining.requests,
        32
    );
    assert_eq!(
        requester
            .sync
            .transport()
            .sent_blob_requests
            .load(std::sync::atomic::Ordering::Acquire),
        2
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache")).unwrap().count(),
        0
    );
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

/// Actual native store factory, including Studio's outer ProtectedBlobs adapter. Retention
/// regressions must not assume the raw KeptBlobStore is the desktop's installed trait object.
fn attach_protected_kept<T: MeshTransport>(
    server: &mut Server<T, ChaCha20Rng>,
    root: &std::path::Path,
) -> crate::store::ServerStore {
    let store = crate::store::ServerStore::open(
        root,
        b"native attachment",
        &mut ChaCha20Rng::seed_from_u64(92),
    )
    .unwrap();
    let mut blobs = store.blob_store(&hex::encode(server.group_id())).unwrap();
    for cid in server.sync.blob_cids() {
        blobs.put(&server.sync.get_blob(&cid).unwrap()).unwrap();
    }
    server.set_blob_store(blobs);
    store
}
async fn drive_keep<T: MeshTransport + 'static>(
    server: &mut Server<T, ChaCha20Rng>,
    transfers: &mut FileTransfers,
    mut result: oneshot::Receiver<Result<(), String>>,
) {
    for _ in 0..12 {
        if let Ok(outcome) = result.try_recv() {
            outcome.expect("native attached Keep must complete");
            return;
        }
        let step = bounded(transfers.next())
            .await
            .expect("native attached Keep continuation");
        transfers.complete(server, step);
    }
    panic!("bounded local Keep did not finish");
}

async fn drive_download<T: MeshTransport + 'static>(
    server: &mut Server<T, ChaCha20Rng>,
    transfers: &mut FileTransfers,
    mut result: oneshot::Receiver<Result<DownloadReady, String>>,
) -> Result<DownloadReady, String> {
    for _ in 0..24 {
        if let Ok(outcome) = result.try_recv() {
            return outcome;
        }
        let step = bounded(transfers.next())
            .await
            .expect("private download continuation");
        transfers.complete(server, step);
    }
    panic!("private download exceeded its bounded test steps");
}

#[tokio::test]
async fn private_download_uses_real_acquisition_and_protected_store_without_publication() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        entered: _entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    let _store = attach_protected_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    gate.add_permits(8);
    let mut transfers = isolated_transfers();
    let (_cancel, watch) = watch::channel(false);
    let (reply, result) = oneshot::channel();
    transfers.download(
        &mut requester,
        cid.as_bytes().to_vec(),
        None,
        RequestCancellation::new(watch, None),
        reply,
    );
    let ready = drive_download(&mut requester, &mut transfers, result)
        .await
        .unwrap();
    assert!(ready.is_current());
    assert_eq!(ready.cid(), cid);
    assert!(requester.sync.kept_files().files.is_empty());
    assert!(requester.sync.kept_files().transfers.is_empty());
    assert!(requester.sync.automatic_keep_candidate(None).is_none());
    for chunk in &ready.manifest.chunks {
        assert!(!requester.sync.has_blob(&chunk.ciphertext_cid));
        assert!(requester.sync.get_blob(&chunk.ciphertext_cid).is_none());
        let bytes = requester
            .sync
            .read_download_export(
                &ready.lease,
                &chunk.ciphertext_cid,
                kept_ciphertext_bound(chunk).unwrap(),
            )
            .unwrap();
        requester.sync.open_file(&bytes, chunk).unwrap();
    }
    assert!(requester.sync.downloads().unwrap()[0].checked);
    requester.sync.cancel_download(ready.id()).unwrap();
    assert!(!ready.is_current());
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn interrupted_private_download_preserves_original_plan_and_spend_on_explicit_resume() {
    let Pair {
        mut requester,
        holder,
        cid,
        gate,
        mut entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let (source, _events, source_task) = spawn(holder);
    let mut transfers = isolated_transfers();
    let (cancel, watch) = watch::channel(false);
    let (reply, result) = oneshot::channel();
    transfers.download(
        &mut requester,
        cid.as_bytes().to_vec(),
        None,
        RequestCancellation::new(watch, None),
        reply,
    );
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    let lower = bounded(entered.recv()).await.unwrap();
    let before = requester.sync.downloads().unwrap().remove(0);
    cancel.send_replace(true);
    let step = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, step);
    assert!(bounded(result).await.unwrap().is_err() && lower.is_cancelled());
    requester.set_blob_store(Box::new(catcoms_storage::MemoryBlobStore::new()));
    attach_kept(&mut requester, dir.path());
    let after = requester.sync.downloads().unwrap().remove(0);
    assert_eq!(after.state.remaining, before.state.remaining);
    assert_eq!(after.state.grant, before.state.grant);
    assert!(!after.state.running && !after.checked);
    assert!(requester.sync.automatic_keep_candidate(None).is_none());
    gate.add_permits(8);
    let (_cancel, watch) = watch::channel(false);
    let (reply, result) = oneshot::channel();
    transfers.download(
        &mut requester,
        cid.as_bytes().to_vec(),
        Some(before.id),
        RequestCancellation::new(watch, None),
        reply,
    );
    let ready = drive_download(&mut requester, &mut transfers, result)
        .await
        .unwrap();
    assert!(ready.is_current() && ready.id() == before.id);
    assert_eq!(
        requester.sync.downloads().unwrap()[0].state.grant,
        before.state.grant
    );
    assert!(
        requester.sync.downloads().unwrap()[0]
            .state
            .remaining
            .requests
            < before.state.remaining.requests
    );
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn private_export_cannot_survive_actual_actor_freeze_and_thaw() {
    let Pair {
        mut holder,
        cid,
        entered: _entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    let store = attach_protected_kept(&mut holder, dir.path());
    let vault = Arc::new(tokio::sync::Mutex::new(Some(store)));
    let (actor, mut events, task) = spawn(holder);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_cancel, watch) = watch::channel(false);
    let origin = RequestCancellation::new(watch, None);
    let ready = bounded(actor.acquire_download(cid.as_bytes().to_vec(), None, origin.clone()))
        .await
        .unwrap();
    let slots = Arc::new(Semaphore::new(1));
    let ready = ready.with_export_custody(Arc::new(slots.clone().try_acquire_owned().unwrap()));
    assert!(ready.is_current());
    let preparing = bounded(actor.prepare_shutdown()).await.unwrap();
    let frozen = bounded(
        preparing.save_and_freeze(crate::shutdown::ShutdownLease::new(
            vault.lock_owned().await,
            77,
            (),
        )),
    )
    .await
    .unwrap();
    assert!(
        !ready.is_current() && !ready.invocation_current(),
        "old native publication/response observers retire before freeze acknowledgement"
    );
    drop(frozen);
    assert!(
        bounded(actor.read_download_chunk(ready.clone(), 0, origin.clone()))
            .await
            .is_err()
    );
    assert!(!ready.is_current(), "thaw cannot revive old export custody");
    let fresh =
        bounded(actor.acquire_download(cid.as_bytes().to_vec(), Some(ready.id()), origin.clone()))
            .await
            .unwrap()
            .with_export_custody(Arc::new(()));
    assert!(fresh.is_current());
    assert!(bounded(actor.read_download_chunk(fresh, 0, origin))
        .await
        .is_ok());
    actor.shutdown().await;
    bounded(task).await.unwrap();
    bounded(drain).await.unwrap();
}

#[tokio::test]
async fn private_plaintext_reply_retains_export_slot_after_native_waiter_drops_its_ready() {
    let Pair {
        mut holder,
        cid,
        entered: _entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    let _store = attach_protected_kept(&mut holder, dir.path());
    let (actor, mut events, task) = spawn(holder);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let (_cancel, watch) = watch::channel(false);
    let origin = RequestCancellation::new(watch, None);
    let ready = bounded(actor.acquire_download(cid.as_bytes().to_vec(), None, origin.clone()))
        .await
        .unwrap();
    let slots = Arc::new(Semaphore::new(1));
    let ready = ready.with_export_custody(Arc::new(slots.clone().try_acquire_owned().unwrap()));
    let (reply, plaintext) = oneshot::channel();
    actor
        .cmd_tx
        .send(AppCommand::ReadDownloadChunk {
            ready,
            index: 0,
            cancel: origin,
            reply,
        })
        .await
        .unwrap();
    bounded(actor.downloads()).await.unwrap(); // FIFO proves the actor finished the read.
    assert_eq!(
        slots.available_permits(),
        0,
        "unconsumed plaintext owns the slot after queued Ready retires"
    );
    drop(plaintext);
    assert_eq!(slots.available_permits(), 1);
    actor.shutdown().await;
    bounded(task).await.unwrap();
    bounded(drain).await.unwrap();
}

#[tokio::test]
async fn private_acquisition_publishes_current_intent_and_pause_before_the_save_reply() {
    let Pair {
        mut requester,
        holder,
        cid,
        mut entered,
        ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    attach_kept(&mut requester, dir.path());
    let resolved = file_resolution::resolve(&requester.files(), &cid).unwrap();
    let (plan, _) = kept_plan(resolved.variants[0].0.file_ref.clone(), cid).unwrap();
    let keep = requester
        .sync
        .begin_keep_with_listing(plan, resolved.version)
        .unwrap();
    requester.sync.pause_keep(keep).unwrap();
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, mut events, task) = spawn(requester);
    let (_cancel, watch) = watch::channel(false);
    let origin = RequestCancellation::new(watch, None);
    let saving = tokio::spawn({
        let actor = actor.clone();
        let origin = origin.clone();
        async move {
            actor
                .acquire_download(cid.as_bytes().to_vec(), None, origin)
                .await
        }
    });
    let lower = bounded(entered.recv()).await.unwrap();
    bounded(async {
        while !matches!(events.recv().await.unwrap().event, AppEvent::FilesUpdated) {}
    })
    .await;
    let before = bounded(actor.downloads()).await.unwrap().remove(0);
    assert!(before.state.running && !saving.is_finished());
    assert!(
        !actor.kept_files().await.transfers[0].running,
        "same-CID Keep does not mask private acquisition progress"
    );
    while events.try_recv().is_ok() {}
    actor
        .set_download_paused(before.id, before.state.intent, true)
        .await
        .unwrap();
    bounded(async {
        while !matches!(events.recv().await.unwrap().event, AppEvent::FilesUpdated) {}
    })
    .await;
    assert!(bounded(saving).await.unwrap().is_err() && lower.is_cancelled());
    let paused = actor.downloads().await.unwrap().remove(0);
    assert!(paused.state.user_paused && !paused.state.running);
    assert_eq!(paused.state.intent, before.state.intent + 1);
    assert_eq!(paused.state.remaining, before.state.remaining);
    actor
        .set_download_paused(paused.id, paused.state.intent, false)
        .await
        .unwrap();
    while events.try_recv().is_ok() {}
    let resuming = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .acquire_download(cid.as_bytes().to_vec(), Some(paused.id), origin)
                .await
        }
    });
    let lower = bounded(entered.recv()).await.unwrap();
    bounded(async {
        while !matches!(events.recv().await.unwrap().event, AppEvent::FilesUpdated) {}
    })
    .await;
    let resumed = actor.downloads().await.unwrap().remove(0);
    assert!(resumed.state.running && !resuming.is_finished());
    assert_eq!(resumed.state.intent, paused.state.intent + 1);
    assert_eq!(resumed.state.grant, before.state.grant);
    actor
        .set_download_paused(resumed.id, resumed.state.intent, true)
        .await
        .unwrap();
    assert!(bounded(resuming).await.unwrap().is_err() && lower.is_cancelled());
    actor.shutdown().await;
    drop(events);
    bounded(task).await.unwrap();
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}
#[tokio::test]
async fn protected_kept_native_factory_preserves_saved_plan_resume_and_check() {
    let Pair {
        mut holder, cid, ..
    } = pair().await;
    let dir = tempfile::tempdir().unwrap();
    let _store = attach_protected_kept(&mut holder, dir.path());
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    drive_keep(&mut holder, &mut transfers, result).await;
    let saved = holder.sync.kept_files();
    assert!(saved.files[0].checked);
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    drive_keep(&mut holder, &mut transfers, result).await;
    assert_eq!(holder.sync.kept_files().files, saved.files);
}
#[tokio::test]
async fn protected_kept_native_factory_preserves_whole_variant_fallback_and_saved_authority() {
    let Pair {
        mut holder, cid, ..
    } = pair().await;
    let good = add_bad_wrapped_variant(&mut holder, &cid).await;
    let dir = tempfile::tempdir().unwrap();
    let _store = attach_protected_kept(&mut holder, dir.path());
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    drive_keep(&mut holder, &mut transfers, result).await;
    let saved = holder.sync.kept_files();
    assert!(saved.files[0].checked);
    assert_eq!(
        saved.files[0].plan.version,
        crate::file_manifest_version(&good.encode())
    );
    assert_eq!(holder.sync.kept_saved_plans(&cid).unwrap().len(), 2);
    assert_eq!(saved.transfers[0].grant, 1);
    let (reply, result) = oneshot::channel();
    transfers.keep(&mut holder, cid.as_bytes().to_vec(), None, reply);
    drive_keep(&mut holder, &mut transfers, result).await;
    assert_eq!(holder.sync.kept_files().files, saved.files);
    assert_eq!(
        holder.sync.kept_files().transfers[0].transfer,
        saved.transfers[0].transfer
    );
}

#[tokio::test]
async fn retired_actor_generation_rejects_old_result_and_resume_never_revives_lower_authority() {
    let Pair {
        mut requester,
        mut holder,
        cid,
        data,
        gate,
        mut entered,
        ..
    } = pair().await;
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let lower = bounded(entered.recv()).await.unwrap();
    assert!(!lower.is_cancelled());
    transfers.retire();
    assert!(
        lower.is_cancelled(),
        "retire must be synchronous at queued driver admission"
    );
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS - 1);
    transfers.resume();
    assert!(
        lower.is_cancelled(),
        "fresh actor generation cannot revive old lower work"
    );
    let completion = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, completion);
    assert!(bounded(result).await.unwrap().is_err());
    assert!(
        requester.sync.blob_cids().is_empty(),
        "retired completion cannot cache bytes"
    );
    assert_eq!(
        transfers.slots.available_permits(),
        SERVER_ATTEMPTS - 1,
        "actual lower observer retains its original credit"
    );
    drop(lower);
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS);

    let (reply, mut result) = oneshot::channel();
    gate.add_permits(1);
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let fresh = bounded(entered.recv()).await.unwrap();
    assert!(!fresh.is_cancelled());
    let bytes = bounded(async {
        loop {
            tokio::select! {
                result = &mut result => break result.unwrap().unwrap().0,
                step = transfers.next(), if !transfers.is_empty() => transfers.complete(&mut requester, step.unwrap()),
                _ = holder.sync_once() => {},
            }
        }
    })
    .await;
    assert_eq!(bytes, data);
}

#[tokio::test]
async fn stopped_actor_revokes_file_authority_before_ack_while_close_event_sink_is_full() {
    let Pair {
        requester,
        holder,
        cid,
        mut entered,
        gate,
        ..
    } = pair().await;
    let submissions = requester.sync.transport().sent_blob_requests.clone();
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, mut events, task) = spawn(requester);
    let reading = tokio::spawn({
        let actor = actor.clone();
        async move { actor.fetch_file_chunk(cid.as_bytes().to_vec(), 0).await }
    });
    let lower = bounded(entered.recv()).await.unwrap();
    // A query is a real actor boundary after startup. Then fill only its actual event sink;
    // no fixed sleep and no replacement actor/transport/stop implementation is involved.
    assert!(bounded(actor.file_head(cid.as_bytes().to_vec()))
        .await
        .is_some());
    while events.try_recv().is_ok() {}
    let event_sender = actor.test_events.upgrade().unwrap();
    while event_sender
        .try_send(TracedEvent {
            trace: Trace::NONE,
            event: AppEvent::SnapshotNeeded,
        })
        .is_ok()
    {}
    assert_eq!(event_sender.capacity(), 0);
    drop(event_sender);
    bounded(actor.stop_and_wait()).await.unwrap();
    assert!(
        lower.is_cancelled(),
        "native ACK cannot precede file-effect retirement"
    );
    assert!(
        !task.is_finished(),
        "full event sink really retains the stopped actor"
    );
    assert_eq!(submissions.load(std::sync::atomic::Ordering::Acquire), 0);
    gate.add_permits(1);
    // Lower accounting remains genuinely owned even while its effect is dead.
    assert!(lower.keepalive().is_some());
    let mut cancelled = lower.clone();
    bounded(cancelled.cancelled()).await;
    drop(events);
    bounded(task).await.unwrap();
    assert!(bounded(reading).await.unwrap().is_err());
    assert_eq!(submissions.load(std::sync::atomic::Ordering::Acquire), 0);
    drop(lower);
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn observed_mls_removal_revokes_queued_file_request_before_late_driver_admission() {
    let Pair {
        mut requester,
        mut holder,
        cid,
        mut entered,
        gate,
        ..
    } = pair().await;
    // This manually driven owner must opt into real membership-control delivery, as spawn does.
    requester.subscribe_control().await.unwrap();
    let mut transfers = isolated_transfers();
    let (reply, result) = oneshot::channel();
    transfers.chunk(&mut requester, cid.as_bytes().to_vec(), 0, None, reply);
    let lower = bounded(entered.recv()).await.unwrap();
    let scope = requester.sync.file_effect_authority().unwrap();
    assert!(!lower.is_cancelled());
    holder
        .remove_member(&requester.my_fingerprint())
        .await
        .unwrap();
    bounded(async {
        while scope.authorized() {
            requester.sync_once().await.unwrap();
        }
    })
    .await;
    assert!(
        lower.is_cancelled(),
        "observed MLS removal must fence the actual queued driver"
    );
    assert!(
        requester.sync.file_effect_authority().is_err(),
        "removed membership cannot mint a fresh live scope"
    );
    assert_eq!(
        requester
            .sync
            .transport()
            .sent_blob_requests
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
    gate.add_permits(1);
    let finished = bounded(transfers.next()).await.unwrap();
    transfers.complete(&mut requester, finished);
    assert!(bounded(result).await.unwrap().is_err());
    assert!(requester.sync.blob_cids().is_empty());
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS - 1);
    drop(lower);
    assert_eq!(transfers.slots.available_permits(), SERVER_ATTEMPTS);
}

#[tokio::test]
async fn cancelled_actor_freeze_mints_fresh_file_work_without_reviving_its_old_request() {
    let Pair {
        requester,
        holder,
        cid,
        data,
        mut entered,
        gate,
        ..
    } = pair().await;
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, _events, task) = spawn(requester);
    let old_read = tokio::spawn({
        let actor = actor.clone();
        async move { actor.fetch_file_chunk(cid.as_bytes().to_vec(), 0).await }
    });
    let old = bounded(entered.recv()).await.unwrap();
    let ready = bounded(actor.prepare_shutdown()).await.unwrap();
    assert!(
        old.is_cancelled(),
        "freeze's first Ready/lease await cannot keep old work authorized"
    );
    drop(ready);
    assert!(bounded(old_read).await.unwrap().is_err());
    // The same actor is usable again, but none of its old lower/request observers are revived.
    assert!(bounded(actor.file_head(cid.as_bytes().to_vec()))
        .await
        .is_some());
    let fresh_read = tokio::spawn({
        let actor = actor.clone();
        async move { actor.fetch_file_chunk(cid.as_bytes().to_vec(), 0).await }
    });
    let fresh = bounded(entered.recv()).await.unwrap();
    assert!(!fresh.is_cancelled());
    assert!(old.is_cancelled());
    gate.add_permits(1);
    assert_eq!(bounded(fresh_read).await.unwrap().unwrap().0, data);
    drop((old, fresh));
    actor.shutdown().await;
    bounded(task).await.unwrap();
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}
#[test]
fn incoming_source_and_outgoing_file_jobs_share_one_irreversible_actor_generation() {
    let mut transfers = FileTransfers::new();
    let original = transfers.authority();
    let repeated = transfers.authority();
    assert!(original.authorized() && repeated.authorized());
    transfers.retire();
    assert!(!original.authorized() && !repeated.authorized());
    transfers.resume();
    assert!(!original.authorized() && !repeated.authorized());
    let fresh = transfers.authority();
    assert!(fresh.authorized());
    drop(transfers);
    assert!(!fresh.authorized());
}

#[tokio::test]
async fn queued_media_range_keeps_original_invocation_across_cancelled_freeze() {
    let Pair {
        requester,
        holder,
        cid,
        data,
        mut entered,
        gate,
        ..
    } = pair().await;
    let version = requester.file_head(&cid).unwrap().manifest_version;
    let sent = requester.sync.transport().sent_blob_requests.clone();
    let (source, _source_events, source_task) = spawn(holder);
    let (actor, _events, task) = spawn(requester);
    let ready = bounded(actor.prepare_shutdown()).await.unwrap();
    let (original, receiver) = watch::channel(false);
    let (reply, result) = oneshot::channel();
    // Send into the real bounded actor queue while its shutdown lease is frozen. The public
    // cancellable method constructs this exact command; the actor cannot inspect it yet.
    actor
        .cmd_tx
        .send(AppCommand::ReadFileRange {
            cid: cid.as_bytes().to_vec(),
            expected_manifest_version: version,
            start: 0,
            max_len: CHUNK_BYTES,
            cancel: Some(RequestCancellation::new(receiver, None)),
            reply,
        })
        .await
        .unwrap();
    original.send_replace(true);
    drop(ready);
    assert!(bounded(result)
        .await
        .unwrap()
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(sent.load(std::sync::atomic::Ordering::Acquire), 0);
    assert!(
        entered.try_recv().is_err(),
        "old command must not reach the lower driver"
    );
    let (_fresh, receiver) = watch::channel(false);
    gate.add_permits(1);
    assert_eq!(
        bounded(actor.read_file_range_cancellable(
            cid.as_bytes().to_vec(),
            version,
            0,
            CHUNK_BYTES,
            Some(RequestCancellation::new(receiver, None))
        ))
        .await
        .unwrap()
        .bytes,
        data
    );
    actor.shutdown().await;
    bounded(task).await.unwrap();
    source.shutdown().await;
    bounded(source_task).await.unwrap();
}

#[tokio::test]
async fn native_retirement_after_real_aead_refuses_storage_and_bad_verdicts_in_both_paths() {
    for (local, bad_key, at_clock) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let Pair {
            mut requester,
            holder,
            cid,
            gate,
            entered: _entered,
            clock_hook,
            ..
        } = pair().await;
        if bad_key {
            add_bad_wrapped_variant(&mut requester, &cid).await;
        }
        let directory = tempfile::tempdir().unwrap();
        attach_kept(&mut requester, directory.path());
        if local {
            for blob in holder.sync.blob_cids() {
                requester
                    .sync
                    .put_blob(&holder.sync.get_blob(&blob).unwrap())
                    .unwrap();
            }
        }
        let before_cache = requester.sync.blob_cids();
        let (source, _events, source_task) = spawn(holder);
        gate.add_permits(8);
        let (original, receiver) = watch::channel(false);
        let mut transfers = isolated_transfers();
        transfers.before_storage_admission = Some(Box::new(move || {
            if at_clock {
                // Arm after actual AEAD; validate_keep_work's NEXT sample retires the caller.
                // Checking only before that validation used to admit storage/bad-key effects.
                *clock_hook.lock().unwrap() = Some(Box::new(move || {
                    original.send_replace(true);
                }));
            } else {
                original.send_replace(true);
            }
        }));
        let (reply, mut result) = oneshot::channel();
        transfers.keep(
            &mut requester,
            cid.as_bytes().to_vec(),
            Some(RequestCancellation::new(receiver, None)),
            reply,
        );
        let admitted = requester.sync.kept_files().transfers[0].clone();
        let error = bounded(async {
            loop { tokio::select! {
                result = &mut result => break result.unwrap().unwrap_err(),
                step = transfers.next(), if !transfers.is_empty() => transfers.complete(&mut requester, step.unwrap()),
            }}
        }).await;
        assert!(
            error.contains("cancelled"),
            "path local={local}, bad_key={bad_key}, at_clock={at_clock}: {error}"
        );
        assert!(
            transfers.before_storage_admission.is_none(),
            "actual AEAD completion boundary was reached"
        );
        assert_eq!(requester.sync.blob_cids(), before_cache);
        let inventory = requester.sync.kept_files();
        assert!(inventory.files.is_empty());
        assert!(inventory
            .transfers
            .iter()
            .all(|entry| entry.verified_chunks == 0));
        assert_eq!(
            inventory.transfers[0].plan, admitted.plan,
            "retired AEAD cannot select another candidate"
        );
        assert_eq!(inventory.transfers[0].intent, admitted.intent);
        assert_eq!(inventory.transfers[0].grant, admitted.grant);
        assert!(
            !inventory.transfers[0].blocked,
            "retired AEAD cannot block a candidate"
        );
        assert_eq!(transfers.jobs.available_permits(), SERVER_ATTEMPTS);
        source.shutdown().await;
        bounded(source_task).await.unwrap();
    }
}
