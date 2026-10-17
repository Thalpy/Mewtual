//! Bounded file reads whose network suspension never borrows the Server. The actor prepares and
//! commits each step; workers own only an opaque signed request, cancellation and reply lifetime.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, OnceLock};

use catcoms_rt::SharedRequestKeepalive;
use catcoms_sync::CompletedBlobFetch;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use super::*;
use crate::{file_resolution, CHUNK_BYTES};

/// Also bounds ready-but-unconsumed responses. These permits follow each transport attempt,
/// including requests whose application deadline expired before the lower stream terminated.
const SERVER_ATTEMPTS: usize = 4;
const PROCESS_ATTEMPTS: usize = 8;
/// This layer's own per-attempt budget, deliberately **longer** than the transport's.
///
/// It used to be 8 s against libp2p's 10 s, which is the wrong way round and cost more than the
/// two seconds suggests. libp2p request-response exposes no outbound cancel: once `send_request`
/// reaches the swarm the stream lives until its own timeout whatever this layer decides. So an
/// app deadline *under* the transport's does not stop the bytes, it only stops waiting for them,
/// and the retry it triggers then competes with an abandoned request still pushing data down the
/// same path. On the single NAT-punched route two otherwise-unreachable members share, that is
/// self-inflicted congestion on the only link there is.
///
/// Above the transport timeout, the transport's terminal event is always what ends an attempt,
/// and this is only a backstop for a driver that never reports one.
const ATTEMPT_MS: u64 = catcoms_rt::REQUEST_TIMEOUT_MS + 2_000;
const READ_MS: u64 = 60_000;

/// Exhausted-candidate rounds one chunk may spend before the read gives up.
///
/// `SERVER_ATTEMPTS` is a *concurrency* permit count and was never a retry budget, which is why
/// a group with one other member online got exactly one attempt per chunk. `READ_MS` is the real
/// bound: this cap only stops a pathological provider from making the actor rebuild a candidate
/// list thousands of times inside that window.
const MAX_ROUNDS: u8 = 6;

/// Pause between exhausted-candidate rounds.
///
/// Long enough that a retry is not simply a second copy of the burst that just failed, short
/// enough that six of them still fit inside `READ_MS` beside their attempts.
const ROUND_BACKOFF_MS: u64 = 750;

/// Pause before re-offering a job whose only providers already have a bulk request in flight.
const BUSY_PEER_BACKOFF_MS: u64 = 250;

/// Pages one chunk may be served in.
///
/// Because an accepted page refreshes the read deadline, something has to stop a provider that
/// dribbles a byte at a time from extending a read indefinitely. A chunk is at most 8 MiB and a
/// page opens at 64 KiB, so a well-behaved transfer uses well under a hundred; this only bites
/// on a provider that is not cooperating.
const MAX_PAGES_PER_CHUNK: usize = 1024;

/// The error this layer produces when its own attempt budget expires before the transport
/// terminates. Named so the retry classifier and the producer cannot drift apart silently.
const ATTEMPT_EXPIRED_ERROR: &str = "file provider timed out";

fn process_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(PROCESS_ATTEMPTS)))
        .clone()
}

#[derive(Debug)]
struct AttemptAccounting {
    _server: OwnedSemaphorePermit,
    _process: OwnedSemaphorePermit,
    _caller: Option<SharedRequestKeepalive>,
}

/// Dropping a watch sender does not make `is_cancelled()` true. Signal explicitly even during
/// abort/unwind so a request still queued behind a busy transport cannot be admitted afterward.
struct CancelOnDrop(watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

enum ReadReply {
    Chunk(oneshot::Sender<ChunkResult>),
    Range(oneshot::Sender<Result<FileRange, String>>),
    Keep(oneshot::Sender<Result<(), String>>),
}

impl ReadReply {
    fn is_closed(&self) -> bool {
        match self {
            Self::Chunk(reply) => reply.is_closed(),
            Self::Range(reply) => reply.is_closed(),
            Self::Keep(reply) => reply.is_closed(),
        }
    }

    async fn closed(&mut self) {
        match self {
            Self::Chunk(reply) => reply.closed().await,
            Self::Range(reply) => reply.closed().await,
            Self::Keep(reply) => reply.closed().await,
        }
    }

    fn fail(self, error: String) {
        match self {
            Self::Chunk(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Range(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Keep(reply) => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

struct RangeState {
    start: u64,
    end: u64,
    total_size: u64,
    mime: String,
    bytes: Vec<u8>,
    provider: Option<String>,
}

struct KeepState {
    token: u64,
    manifest: crate::FileManifest,
    hasher: catcoms_storage::CidHasher,
    existing: bool,
}

pub(super) struct ReadJob {
    cid: Cid,
    version: [u8; 32],
    epoch: u64,
    index: usize,
    deadline: u64,
    cancel: Option<RequestCancellation>,
    reply: ReadReply,
    range: Option<RangeState>,
    keep: Option<KeepState>,
    candidates: Option<VecDeque<(FileRef, PeerId)>>,
    error: String,
    /// Ciphertext accumulated for the current chunk by paged fetching.
    ///
    /// This is what turns a failed attempt from lost work into a pause. Because a blob is
    /// content-addressed and immutable, the offset it resumes at is meaningful against *any*
    /// holder, so a retry may even land on a different provider and continue where the last one
    /// stopped. Nothing here is trusted: every page is individually signed for this request and
    /// window, and the whole is re-hashed against the chunk's content address before a byte of
    /// it is opened or stored.
    staged: Vec<u8>,
    /// Which encrypted variant `staged` holds bytes of, so a retry that picks a different one
    /// restarts rather than splicing two ciphertexts together.
    staged_cid: Option<Cid>,
    /// Page size for the next request, measured rather than declared. See
    /// `ChannelSync::blob_page_ceiling` for why the path class cannot supply it.
    page_size: usize,
    /// Pages already accepted for this chunk, bounding a provider that dribbles.
    pages: u16,
    /// Whether this provider is believed to understand paging. Cleared when one answers empty
    /// to a page request, which on that kind means it predates the grammar.
    paging: bool,
    /// Exhausted-candidate rounds already spent on the current chunk.
    ///
    /// The candidate list is the cross product of missing references and connected providers, so
    /// in the shape the product is actually deployed in (one other member online, one encrypted
    /// variant) it holds exactly **one** entry. Draining it used to end the job, which made
    /// `READ_MS` unreachable: a sixty-second per-chunk budget that no code path could spend,
    /// because the single attempt inside it was capped at `ATTEMPT_MS`. One dropped packet at
    /// the wrong moment failed the whole download with no second try.
    rounds: u8,
    /// Whether the last failure was one a fresh attempt could plausibly survive.
    ///
    /// Set for transport and deadline failures only. An empty answer ("I do not hold this"), a
    /// decryption failure or a changed manifest are all answers, not accidents: re-asking the
    /// same provider inside the same deadline cannot change them, and doing so would spin.
    retryable: bool,
}

impl ReadJob {
    fn current<T: MeshTransport, R: CryptoRngCore>(&self, server: &Server<T, R>) -> bool {
        !self.reply.is_closed()
            && !self
                .cancel
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
            && server.epoch() == self.epoch
            && if self.keep.as_ref().is_some_and(|keep| keep.existing) {
                server
                    .sync
                    .kept_files()
                    .files
                    .iter()
                    .any(|copy| copy.plan.cid == self.cid && copy.plan.version == self.version)
            } else {
                server
                    .file_head(&self.cid)
                    .is_some_and(|head| head.manifest_version == self.version)
            }
    }

    fn fail<T: MeshTransport, R: CryptoRngCore>(self, server: &mut Server<T, R>, error: String) {
        // The terminal edge of a download, and until now the only thing a failed transfer left
        // behind was an unattributed transport warning from two layers down. `catcoms_app` is at
        // `debug` in the GUI file filter, so this is the line that actually reaches whoever is
        // reading a shared log. Debug rather than warn because ordinary cancellation (the user
        // scrolled a preview out of view) comes through here too and is not a defect.
        tracing::debug!(
            cid = ?self.cid,
            chunk = self.index,
            kind = if self.keep.is_some() {
                "keep"
            } else if self.range.is_some() {
                "range"
            } else {
                "chunk"
            },
            %error,
            "file transfer failed"
        );
        if let Some(keep) = &self.keep {
            let _ = server.sync.abort_keep(keep.token);
        }
        self.reply.fail(error);
    }

    /// A range retains at most CHUNK_BYTES, even when its window crosses two encrypted chunks.
    fn accept<T: MeshTransport, R: CryptoRngCore>(
        mut self,
        server: &mut Server<T, R>,
        bytes: Vec<u8>,
        provider: Option<String>,
    ) -> Option<Self> {
        if let Some(keep) = self.keep.as_mut() {
            keep.hasher.update(&bytes);
            if self.index + 1 < keep.manifest.chunks.len() {
                self.index += 1;
                self.candidates = None;
                // Rounds and staged bytes are a per-chunk allowance, like the deadline beside
                // them. Page size is not: it is what this link has been measured to carry, and
                // that measurement is worth keeping across chunks of the same read.
                self.rounds = 0;
                self.retryable = false;
                self.staged = Vec::new();
                self.staged_cid = None;
                self.pages = 0;
                // Keep uses a per-chunk deadline; the finite manifest bounds total work.
                self.deadline = server
                    .runtime_clock()
                    .monotonic_ms()
                    .saturating_add(READ_MS);
                return Some(self);
            }
            if keep.hasher.cid() != self.cid {
                self.fail(
                    server,
                    "kept file failed whole-file content verification".into(),
                );
                return None;
            }
            let result = server
                .sync
                .finish_keep(keep.token)
                .map_err(|e| e.to_string());
            if result.is_err() {
                let _ = server.sync.abort_keep(keep.token);
            }
            if let ReadReply::Keep(reply) = self.reply {
                let _ = reply.send(result);
            }
        } else if let Some(range) = self.range.as_mut() {
            let base = self.index as u64 * CHUNK_BYTES as u64;
            let lo = range.start.saturating_sub(base).min(bytes.len() as u64) as usize;
            let hi = range.end.saturating_sub(base).min(bytes.len() as u64) as usize;
            range.bytes.extend_from_slice(&bytes[lo..hi]);
            range.provider = range.provider.take().or(provider);
            if base + (bytes.len() as u64) < range.end {
                self.index += 1;
                self.candidates = None;
                self.rounds = 0;
                self.retryable = false;
                self.staged = Vec::new();
                self.staged_cid = None;
                self.pages = 0;
                return Some(self);
            }
            if let ReadReply::Range(reply) = self.reply {
                let _ = reply.send(Ok(FileRange {
                    bytes: std::mem::take(&mut range.bytes),
                    total_size: range.total_size,
                    mime: range.mime.clone(),
                    provider: range.provider.take(),
                }));
            }
        } else if let ReadReply::Chunk(reply) = self.reply {
            let _ = reply.send(Ok((bytes, provider)));
        }
        None
    }
}

pub(super) enum TransferStep {
    Network(Box<FinishedAttempt>),
    /// Yield local copying between chunks too; a fully cached gigabyte must not monopolize the actor.
    Local(Box<ReadJob>),
}

pub(super) struct FinishedAttempt {
    job: ReadJob,
    reference: FileRef,
    /// The provider this attempt was addressed to, released from `inflight` on completion.
    peer: PeerId,
    /// Whether this attempt asked for a page or for the whole blob.
    paged: bool,
    result: Result<CompletedBlobFetch, String>,
    // A timeout response may be queued while the actual request also remains in libp2p. Holding
    // this token in BOTH places bounds each ownership phase without prematurely refunding either.
    _accounting: SharedRequestKeepalive,
}

pub(super) struct FileTransfers {
    running: JoinSet<TransferStep>,
    slots: Arc<Semaphore>,
    process: Arc<Semaphore>,
    /// Providers with a bulk request already on the wire.
    ///
    /// `SERVER_ATTEMPTS` bounds concurrency across the server, which is the right shape when
    /// those attempts go to different peers. When they go to the *same* peer they share one
    /// path, and on a NAT-punched link that is the only path there is: concurrent multi-megabyte
    /// requests then compete with each other and with the gossip keeping the mapping alive.
    /// One bulk request per provider, `SERVER_ATTEMPTS` across providers.
    inflight: HashSet<PeerId>,
}

impl FileTransfers {
    pub(super) fn new() -> Self {
        Self {
            running: JoinSet::new(),
            slots: Arc::new(Semaphore::new(SERVER_ATTEMPTS)),
            process: process_slots(),
            inflight: HashSet::new(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    pub(super) async fn next(&mut self) -> Option<TransferStep> {
        self.running.join_next().await.and_then(Result::ok)
    }

    fn yield_local(&mut self, job: ReadJob) {
        self.running.spawn(async move {
            tokio::task::yield_now().await;
            TransferStep::Local(Box::new(job))
        });
    }

    /// Re-offer `job` to the actor after `delay`, without the actor waiting for it.
    ///
    /// Sleeping inside `advance` is not available: it runs on the actor and must not block. This
    /// borrows the same `JoinSet` round trip `yield_local` already uses, so a paused job is
    /// ordinary queued transfer work and stays subject to cancellation and the read deadline.
    fn delay_local<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &Server<T, R>,
        job: ReadJob,
        delay_ms: u64,
    ) {
        let clock = server.runtime_clock();
        self.running.spawn(async move {
            clock.sleep(Duration::from_millis(delay_ms)).await;
            TransferStep::Local(Box::new(job))
        });
    }

    pub(super) fn keep<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        raw: Vec<u8>,
        cancel: Option<RequestCancellation>,
        reply: oneshot::Sender<Result<(), String>>,
    ) {
        if reply.is_closed() {
            return;
        }
        if cancel
            .as_ref()
            .is_some_and(RequestCancellation::is_cancelled)
        {
            let _ = reply.send(Err("file keeping cancelled".into()));
            return;
        }
        let result = (|| {
            let cid = Cid::from_bytes(
                raw.as_slice()
                    .try_into()
                    .map_err(|_| "bad content address".to_string())?,
            );
            let existing = server
                .sync
                .kept_files()
                .files
                .into_iter()
                .find(|f| f.plan.cid == cid);
            // A user may explicitly check/repair their own saved manifest even after unlisting.
            // This local authority never enters files(), media heads, or replicated publication.
            let (encoded, manifest, version, recheck) = if let Some(copy) = existing {
                let manifest = crate::FileManifest::decode_or_legacy(&copy.plan.manifest)
                    .map_err(|e| e.to_string())?;
                if manifest.plaintext_cid != cid
                    || crate::file_manifest_version(&copy.plan.manifest) != copy.plan.version
                {
                    return Err("kept manifest is inconsistent".into());
                }
                (copy.plan.manifest, manifest, copy.plan.version, true)
            } else {
                let resolved = file_resolution::resolve(&server.files(), &cid)
                    .ok_or("no unambiguous file manifest in this server's index")?;
                // Prefer one wholly local exact variant; a repaired upload may have a different
                // encryption from the sorted first variant whose original holder is offline.
                let chosen = resolved
                    .variants
                    .iter()
                    .position(|(_, manifest)| {
                        manifest
                            .chunks
                            .iter()
                            .all(|r| server.sync.has_blob(&r.ciphertext_cid))
                    })
                    .unwrap_or(0);
                let (entry, manifest) = resolved.variants[chosen].clone();
                (entry.file_ref, manifest, resolved.version, false)
            };
            let chunks = manifest
                .chunks
                .iter()
                .map(|r| {
                    let padded = catcoms_storage::padded_len(
                        r.size as usize,
                        catcoms_storage::CHUNK_PAD_FLOOR,
                        CHUNK_BYTES,
                    );
                    (
                        r.ciphertext_cid,
                        padded as u64 + catcoms_storage::PAD_FOOTER_BYTES as u64 + 40,
                    )
                })
                .collect();
            let plan = catcoms_storage::kept::KeepPlan {
                cid,
                version: crate::file_manifest_version(&encoded),
                manifest: encoded,
                chunks,
            };
            let token = server.sync.begin_keep(plan).map_err(|e| e.to_string())?;
            Ok::<_, String>((cid, version, token, manifest, recheck))
        })();
        match result {
            Ok((cid, version, token, manifest, recheck)) => {
                let job = ReadJob {
                    cid,
                    version,
                    epoch: server.epoch(),
                    index: 0,
                    deadline: server
                        .runtime_clock()
                        .monotonic_ms()
                        .saturating_add(READ_MS),
                    cancel,
                    reply: ReadReply::Keep(reply),
                    range: None,
                    keep: Some(KeepState {
                        token,
                        manifest,
                        hasher: catcoms_storage::CidHasher::new(),
                        existing: recheck,
                    }),
                    candidates: None,
                    error: "no connected provider supplied the kept file's exact chunk".into(),
                    rounds: 0,
                    retryable: false,
                    staged: Vec::new(),
                    staged_cid: None,
                    page_size: catcoms_sync::MIN_BLOB_PAGE,
                    pages: 0,
                    paging: true,
                };
                self.yield_local(job);
            }
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    pub(super) fn chunk<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        cid: Vec<u8>,
        index: usize,
        cancel: Option<RequestCancellation>,
        reply: oneshot::Sender<ChunkResult>,
    ) {
        self.begin(server, cid, index, None, cancel, ReadReply::Chunk(reply));
    }

    pub(super) fn range<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        cid: Vec<u8>,
        version: [u8; 32],
        start: u64,
        max_len: usize,
        reply: oneshot::Sender<Result<FileRange, String>>,
    ) {
        if max_len > CHUNK_BYTES {
            let _ = reply.send(Err("media range exceeds the response window limit".into()));
            return;
        }
        self.begin(
            server,
            cid,
            (start / CHUNK_BYTES as u64) as usize,
            Some((version, start, max_len)),
            None,
            ReadReply::Range(reply),
        );
    }

    fn begin<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        raw: Vec<u8>,
        index: usize,
        range: Option<([u8; 32], u64, usize)>,
        cancel: Option<RequestCancellation>,
        reply: ReadReply,
    ) {
        if reply.is_closed() {
            return;
        }
        let Ok(raw) = <[u8; 32]>::try_from(raw.as_slice()) else {
            reply.fail("bad content address".into());
            return;
        };
        let cid = Cid::from_bytes(raw);
        let Some(head) = server.file_head(&cid) else {
            reply.fail("no unambiguous file manifest in this server's index".into());
            return;
        };
        let range = if let Some((version, start, len)) = range {
            if version != head.manifest_version {
                reply.fail("file manifest changed after media authorization".into());
                return;
            }
            if start >= head.total_size || len == 0 {
                if let ReadReply::Range(reply) = reply {
                    let _ = reply.send(Ok(FileRange {
                        bytes: Vec::new(),
                        total_size: head.total_size,
                        mime: head.mime,
                        provider: None,
                    }));
                }
                return;
            }
            Some(RangeState {
                start,
                end: start.saturating_add(len as u64).min(head.total_size),
                total_size: head.total_size,
                mime: head.mime,
                bytes: Vec::new(),
                provider: None,
            })
        } else {
            None
        };
        let job = ReadJob {
            cid,
            version: head.manifest_version,
            epoch: server.epoch(),
            index,
            deadline: server
                .runtime_clock()
                .monotonic_ms()
                .saturating_add(READ_MS),
            cancel,
            reply,
            range,
            keep: None,
            candidates: None,
            error: format!("file not available yet; no connected provider supplied chunk {index}"),
            rounds: 0,
            retryable: false,
            staged: Vec::new(),
            staged_cid: None,
            page_size: catcoms_sync::MIN_BLOB_PAGE,
            pages: 0,
            paging: true,
        };
        self.advance(server, job);
    }

    /// Only the actor calls this function. No wait (including semaphore acquisition) occurs here.
    fn advance<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        mut job: ReadJob,
    ) {
        loop {
            if !job.current(server) {
                job.fail(
                    server,
                    "file download cancelled or authorization changed".into(),
                );
                return;
            }
            if server.runtime_clock().monotonic_ms() >= job.deadline {
                job.fail(server, "file download timed out".into());
                return;
            }
            if job.candidates.is_none() {
                let manifests = if let Some(keep) = &job.keep {
                    vec![keep.manifest.clone()]
                } else {
                    let Some(resolved) = file_resolution::resolve(&server.files(), &job.cid) else {
                        job.fail(server, "file manifest changed".into());
                        return;
                    };
                    resolved
                        .variants
                        .into_iter()
                        .map(|(_, manifest)| manifest)
                        .collect()
                };
                if job.index >= manifests[0].chunks.len() {
                    job.fail(server, "chunk is out of range".into());
                    return;
                }
                let mut missing = Vec::new();
                let mut held = None;
                for manifest in manifests {
                    let reference = &manifest.chunks[job.index];
                    if let Some(ciphertext) = server.sync.get_blob(&reference.ciphertext_cid) {
                        if let Ok(bytes) = server.sync.open_file(&ciphertext, reference) {
                            if let Some(keep) = &job.keep {
                                if let Err(error) = server.sync.put_keep(keep.token, &ciphertext) {
                                    job.fail(server, error.to_string());
                                    return;
                                }
                            }
                            held = Some(bytes);
                            break;
                        }
                        job.error = format!("chunk {} could not be decrypted", job.index);
                    } else {
                        missing.push(reference.clone());
                    }
                }
                if let Some(bytes) = held {
                    match job.accept(server, bytes, None) {
                        Some(next) if next.keep.is_some() => {
                            self.yield_local(next);
                            return;
                        }
                        Some(next) => {
                            job = next;
                            continue;
                        }
                        None => return,
                    }
                }
                let peers = server.sync.blob_fetch_peers();
                job.candidates = Some(
                    missing
                        .into_iter()
                        .flat_map(|reference| {
                            peers.iter().map(move |peer| (reference.clone(), *peer))
                        })
                        .collect(),
                );
            }
            // Take the first candidate whose provider is not already carrying a bulk request,
            // rotating the busy ones to the back rather than discarding them: a peer that is
            // busy now is a perfectly good provider a moment later, and dropping it here would
            // reintroduce the single-attempt failure through the other door.
            let live = server.sync.blob_fetch_peers();
            let mut picked = None;
            let mut deferred = false;
            if let Some(candidates) = job.candidates.as_mut() {
                for _ in 0..candidates.len() {
                    let Some((reference, peer)) = candidates.pop_front() else {
                        break;
                    };
                    // A proven or bootstrap candidate can disconnect during another attempt. Drop
                    // it without spending a permit or creating a request that could redial from
                    // remembered routes.
                    if !live.contains(&peer) {
                        continue;
                    }
                    if self.inflight.contains(&peer) {
                        candidates.push_back((reference, peer));
                        deferred = true;
                        continue;
                    }
                    picked = Some((reference, peer));
                    break;
                }
            }
            let Some((reference, peer)) = picked else {
                if deferred {
                    // Every surviving provider is busy. Waiting is the whole point of the
                    // per-peer bound; failing here would make serialization worse than the
                    // concurrency it replaced.
                    self.delay_local(server, job, BUSY_PEER_BACKOFF_MS);
                    return;
                }
                // Candidates exhausted. Rebuilding the list is what makes `READ_MS` reachable:
                // the deadline check at the top of this loop, not the length of a deque, is what
                // should end a read.
                if job.retryable && job.rounds < MAX_ROUNDS {
                    job.rounds += 1;
                    job.retryable = false;
                    job.candidates = None;
                    tracing::debug!(
                        cid = ?job.cid,
                        chunk = job.index,
                        round = job.rounds,
                        "retrying a file chunk after a transport failure"
                    );
                    self.delay_local(server, job, ROUND_BACKOFF_MS);
                    return;
                }
                let error = job.error.clone();
                job.fail(server, error);
                return;
            };
            let permits = self
                .slots
                .clone()
                .try_acquire_owned()
                .ok()
                .zip(self.process.clone().try_acquire_owned().ok());
            let Some((local, global)) = permits else {
                job.fail(server, "file transfers are busy; try again shortly".into());
                return;
            };
            let accounting: SharedRequestKeepalive = Arc::new(AttemptAccounting {
                _server: local,
                _process: global,
                _caller: job.cancel.as_ref().and_then(RequestCancellation::keepalive),
            });
            // Staged bytes belong to one encrypted variant, not to the chunk index. A retry
            // round rebuilds the candidate list from the cross product of every missing
            // reference and every provider, so it can legitimately pick a *different* variant
            // than the one already part-fetched. Resuming at that offset against a different
            // content address would splice two ciphertexts together; the whole-chunk hash would
            // catch it, but only after the transfer had been spent. Start the variant clean.
            if job.staged_cid != Some(reference.ciphertext_cid) {
                if !job.staged.is_empty() {
                    tracing::debug!(
                        cid = ?job.cid,
                        chunk = job.index,
                        discarded = job.staged.len(),
                        "a retry chose a different encrypted variant; restarting the chunk"
                    );
                }
                job.staged.clear();
                job.pages = 0;
                job.staged_cid = Some(reference.ciphertext_cid);
            }
            // Paged by default; the whole-blob grammar is the fallback for a provider that
            // answered a page request with "I do not know this kind".
            let paged = job.paging;
            let prepared = if paged {
                let offset = match u32::try_from(job.staged.len()) {
                    Ok(offset) => offset,
                    Err(_) => {
                        job.fail(server, "staged chunk exceeds the addressable window".into());
                        return;
                    }
                };
                let size = job.page_size.min(server.sync.blob_page_ceiling(peer));
                server
                    .sync
                    .prepare_blob_page(peer, reference.ciphertext_cid, offset, size)
            } else {
                server.sync.prepare_blob_fetch(
                    peer,
                    reference.ciphertext_cid,
                    catcoms_sync::MAX_BOUNDED_BLOB_BYTES,
                )
            };
            let request = match prepared {
                Ok(request) => request,
                Err(error) => {
                    job.error = error.to_string();
                    continue;
                }
            };
            let clock = server.runtime_clock();
            let duration = Duration::from_millis(
                ATTEMPT_MS.min(job.deadline.saturating_sub(clock.monotonic_ms())),
            );
            tracing::debug!(
                cid = ?job.cid,
                chunk = job.index,
                ?peer,
                round = job.rounds,
                budget_ms = duration.as_millis() as u64,
                "requesting a file chunk from a provider"
            );
            self.inflight.insert(peer);
            // Construct the guard BEFORE spawn so aborting even an unpolled task sets the bit.
            let (signal, receiver) = watch::channel(false);
            let guard = CancelOnDrop(signal);
            let cancellation = RequestCancellation::new(receiver, Some(accounting.clone()));
            self.running.spawn(async move {
                let _guard = guard;
                let result = fetch_chunk_or_cancel(job.cancel.clone(), async {
                    tokio::select! {
                        biased;
                        _ = job.reply.closed() => Err("download cancelled".into()),
                        _ = clock.sleep(duration) => Err(ATTEMPT_EXPIRED_ERROR.into()),
                        result = request.fetch(cancellation) => Ok(result),
                    }
                })
                .await;
                TransferStep::Network(Box::new(FinishedAttempt {
                    job,
                    reference,
                    peer,
                    paged,
                    result,
                    _accounting: accounting,
                }))
            });
            return;
        }
    }

    /// Recheck the complete file authorization before `complete_blob_fetch` can write a byte.
    pub(super) fn complete<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        step: TransferStep,
    ) {
        let finished = match step {
            TransferStep::Local(job) => {
                self.advance(server, *job);
                return;
            }
            TransferStep::Network(finished) => *finished,
        };
        let FinishedAttempt {
            mut job,
            reference,
            peer,
            paged,
            result,
            _accounting,
        } = finished;
        // Release the provider before any early return below: a job that fails its own
        // authorization recheck must not leave that peer marked busy for every other transfer.
        self.inflight.remove(&peer);
        if !job.current(server) {
            job.fail(
                server,
                "file download cancelled or authorization changed".into(),
            );
            return;
        }
        // A ready result can wait behind other actor work. Its original read deadline still
        // applies at commit time, rather than only while the worker was suspended on the wire.
        if server.runtime_clock().monotonic_ms() >= job.deadline {
            job.fail(server, "file download timed out".into());
            return;
        }
        if result
            .as_ref()
            .is_err_and(|error| error == "download cancelled")
        {
            job.fail(server, "file download cancelled".into());
            return;
        }
        // Classify before stringifying. Deciding whether to retry by matching on error *prose*
        // would be a decision no compiler can check and no rename would update; the two shapes
        // worth retrying are a transport failure and this layer's own expired attempt budget,
        // and both are available as types right here.
        let attempt_expired = result
            .as_ref()
            .is_err_and(|error| error == ATTEMPT_EXPIRED_ERROR);
        // `outcome` is the whole-chunk ciphertext when one is ready. A page that only advances
        // the chunk sets `resume` instead: the read continues against the same provider, with
        // the bytes it has already earned retained.
        let mut resume = false;
        let outcome = match result {
            Err(error) => {
                job.retryable = attempt_expired;
                Err(error)
            }
            Ok(completed) if paged => {
                match server.sync.authenticate_blob_page(completed) {
                    Ok(catcoms_sync::BlobPageOutcome::Page(page)) => {
                        job.staged.extend_from_slice(&page.bytes);
                        job.pages = job.pages.saturating_add(1);
                        // A page that landed is evidence this link carries that much inside the
                        // budget, so the next one may be larger; the ceiling keeps a relayed
                        // circuit at its opening size however well it is going. This is the
                        // measured half of the sizing, and the only half that can see a punched
                        // path for what it is.
                        job.page_size = job
                            .page_size
                            .saturating_mul(2)
                            .min(catcoms_sync::MAX_BLOB_PAGE);
                        // Progress refreshes the read deadline. `READ_MS` was written as a bound
                        // on one indivisible attempt; with pages it becomes a bound on going
                        // *quiet*, which is the thing actually worth giving up on. The page
                        // count below is what keeps that bounded.
                        job.deadline = server
                            .runtime_clock()
                            .monotonic_ms()
                            .saturating_add(READ_MS);
                        if page.more {
                            if usize::from(job.pages) >= MAX_PAGES_PER_CHUNK {
                                job.fail(
                                    server,
                                    "provider served this chunk in too many pieces".into(),
                                );
                                return;
                            }
                            resume = true;
                            Ok(None)
                        } else {
                            let ciphertext = std::mem::take(&mut job.staged);
                            job.pages = 0;
                            // Each page was signed for its own window, which proves authorship
                            // and placement but not that the run composes to the blob that was
                            // asked for. That is this check, and it happens before a byte is
                            // opened or stored.
                            if Cid::of(&ciphertext) != reference.ciphertext_cid {
                                job.error =
                                    format!("reassembled chunk {} failed verification", job.index);
                                Ok(None)
                            } else {
                                Ok(Some((ciphertext, page.provider)))
                            }
                        }
                    }
                    Ok(catcoms_sync::BlobPageOutcome::Absent) => Ok(None),
                    Ok(catcoms_sync::BlobPageOutcome::Unsupported) => {
                        // A build that predates paging. Restart this chunk on the whole-blob
                        // grammar rather than reading its silence as absence.
                        job.paging = false;
                        job.staged = Vec::new();
                        job.staged_cid = None;
                        job.pages = 0;
                        resume = true;
                        Ok(None)
                    }
                    Err(error) => {
                        job.retryable = matches!(error, catcoms_sync::SyncError::Transport(_));
                        Err(error.to_string())
                    }
                }
            }
            Ok(completed) => server
                .sync
                .authenticate_blob_fetch(completed)
                .map_err(|error| {
                    job.retryable = matches!(error, catcoms_sync::SyncError::Transport(_));
                    error.to_string()
                }),
        };
        job.retryable |= attempt_expired;
        if resume {
            // Re-offer the provider this run is already mid-way through. Continuity is worth
            // preferring: another holder would serve the same bytes, but this one has a warm
            // path and the offset is only meaningful because the blob is immutable.
            if let Some(candidates) = job.candidates.as_mut() {
                candidates.push_front((reference, peer));
            }
            drop(_accounting);
            self.advance(server, job);
            return;
        }
        match outcome {
            Ok(Some((ciphertext, provider))) => {
                match server.sync.open_file(&ciphertext, &reference) {
                    Ok(bytes) => {
                        let stored = if let Some(keep) = &job.keep {
                            server.sync.put_keep(keep.token, &ciphertext)
                        } else {
                            server.sync.put_blob(&ciphertext).map(|_| ())
                        };
                        if let Err(error) = stored {
                            job.fail(server, error.to_string());
                            return;
                        }
                        drop(_accounting);
                        if let Some(next) = job.accept(server, bytes, Some(provider)) {
                            if next.keep.is_some() {
                                self.yield_local(next);
                            } else {
                                self.advance(server, next);
                            }
                        }
                        return;
                    }
                    Err(error) => job.error = error.to_string(),
                }
                // A second provider cannot repair the exact authenticated ciphertext/key pair.
                if let Some(candidates) = job.candidates.as_mut() {
                    candidates.retain(|(r, _)| r.encode() != reference.encode());
                }
            }
            // An empty answer is an *answer*: this provider does not hold the blob, or refused
            // it under its byte budget. Re-asking it inside the same read deadline cannot change
            // that, so this deliberately does not arm a retry round.
            Ok(None) => {}
            // A transport failure or an expired attempt budget is the case a second round exists
            // for. Everything else reaching here (a bad signature, a content-address mismatch, a
            // membership change) is a verdict on the bytes rather than on the wire.
            Err(error) => {
                tracing::debug!(
                    cid = ?job.cid,
                    chunk = job.index,
                    ?peer,
                    retryable = job.retryable,
                    %error,
                    "file chunk attempt failed"
                );
                job.error = error;
            }
        }
        // Drop this completed-result share before retry admission. A genuinely still-live lower
        // request retains its own share; it cannot be refunded by this transition.
        drop(_accounting);
        self.advance(server, job);
    }
}

#[cfg(test)]
mod tests;
