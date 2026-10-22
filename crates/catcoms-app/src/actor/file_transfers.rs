//! Bounded file reads whose network suspension never borrows the Server. The actor prepares and
//! commits each step; workers own only an opaque signed request, cancellation and reply lifetime.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, OnceLock};

use catcoms_rt::SharedRequestKeepalive;
use catcoms_sync::file_service::{
    bridge_reference_variant, BridgeCatalog, BridgeChoice, BridgeError, BridgeStep,
    PreparedBridgePage,
};
use catcoms_sync::CompletedBlobFetch;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use super::*;
use crate::{file_resolution, CHUNK_BYTES};
mod saved;
use saved::SavedRun;

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
/// One absolute owner deadline, not a new relative sleep for every command or network event.
/// Providerless scans do not begin a run, read prefixes or debit durable execution slices.
const AUTOMATIC_KEEP_MS: u64 = 30_000;

/// One source of truth for Keep's requested page length and durable debit. A lying plaintext
/// size cannot make a larger request spend one byte after reaching its declared ciphertext end.
pub(super) fn kept_ciphertext_bound(reference: &FileRef) -> Result<usize, String> {
    if reference.size > CHUNK_BYTES as u64 {
        return Err("chunk exceeds the file size limit".into());
    }
    Ok(catcoms_storage::padded_len(
        reference.size as usize,
        catcoms_storage::CHUNK_PAD_FLOOR,
        CHUNK_BYTES,
    ) + catcoms_storage::PAD_FOOTER_BYTES
        + 40)
}
/// Validate opaque saved plans at the app boundary. Storage can authenticate its journal but
/// cannot interpret FileManifest semantics; recompute every exact digest and ciphertext bound.
fn kept_plan(
    encoded: Vec<u8>,
    cid: Cid,
) -> Result<(catcoms_storage::kept::KeepPlan, crate::FileManifest), String> {
    let manifest = crate::FileManifest::decode_or_legacy(&encoded).map_err(|e| e.to_string())?;
    if manifest.plaintext_cid != cid || manifest.total_size > crate::MAX_FILE_BYTES as u64 {
        return Err("kept manifest is inconsistent".into());
    }
    let chunks = manifest
        .chunks
        .iter()
        .map(|reference| {
            kept_ciphertext_bound(reference).map(|bound| (reference.ciphertext_cid, bound as u64))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let plan = catcoms_storage::kept::KeepPlan {
        cid,
        version: crate::file_manifest_version(&encoded),
        manifest: encoded,
        chunks,
    };
    Ok((plan, manifest))
}

/// Refusal for a "Save as" choice on a Download that already has a journal. The same text as
/// storage's `plan_download_destination` refusal, so the user sees one message whichever check
/// catches it first.
const SAVE_AS_OVER_JOURNAL: &str =
    "This file already has a partly saved copy. Choose Save new copy or Remove in Transfers first.";

/// Undo a Download run whose "Save as" choice could not be written (picker review L2).
///
/// A record this same actor step created (`created`) is released outright: it holds no verified
/// chunks yet, and leaving it would show an "Interrupted" row for a save that never started. An
/// existing record is only paused, keeping its verified progress and allowance. If the release
/// itself fails (most likely the same full or faulted vault that refused the plan write), fall
/// back to pausing so the run never keeps the single active-run slot.
fn abandon_unplanned<T: MeshTransport, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    token: SavedRun,
    created: bool,
) {
    if let (true, SavedRun::Download(run)) = (created, token) {
        if server.sync.cancel_download(run.id()).is_ok() {
            return;
        }
    }
    let _ = token.pause(server);
}

fn kept_request_window(
    reference: &FileRef,
    paged: bool,
    desired: usize,
    staged: usize,
) -> Result<(usize, u64), String> {
    let bound = kept_ciphertext_bound(reference)?;
    if paged {
        let remaining = bound
            .checked_sub(staged)
            .filter(|remaining| *remaining != 0)
            .ok_or("chunk exceeds its declared size")?;
        let requested = desired.min(remaining);
        if requested == 0 {
            return Err("empty file request window".into());
        }
        Ok((requested, requested as u64))
    } else {
        // The legacy request carries only a CID; its local acceptance limit is not sent to the
        // provider. Conservatively prepay a complete released frame, never the claimed size.
        Ok((bound, catcoms_rt::MAX_REQUEST_RESPONSE_FRAME_BYTES as u64))
    }
}

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

fn process_job_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(PROCESS_ATTEMPTS)))
        .clone()
}

/// Whole-job capacity is independent of lower attempts. A job can retain staged bytes, range
/// state or a source search while waiting behind a busy peer, without a network permit yet.
/// This affine owner follows every continuation and is released only after that state drops.
struct JobCustody {
    _server: OwnedSemaphorePermit,
    _process: OwnedSemaphorePermit,
}

#[derive(Debug)]
struct AttemptAccounting {
    _server: OwnedSemaphorePermit,
    _process: OwnedSemaphorePermit,
    _caller: Option<SharedRequestKeepalive>,
}

/// The immutable prepaid deadline follows the request into the transport's own queue. Neither
/// delayed first polling nor a pause/replacement may renew that authority; actual lower custody
/// remains in AttemptAccounting until the transport retires, independently of this effect gate.
#[derive(Debug)]
struct FileRequestAuthority {
    clock: Arc<dyn catcoms_rt::Clock + Send>,
    deadline: u64,
    keep: Option<catcoms_storage::kept::KeepWork>,
    outer: Option<RequestCancellation>,
    actor: watch::Receiver<bool>,
    root: Arc<dyn catcoms_rt::RequestAuthority>,
}
#[async_trait::async_trait]
impl catcoms_rt::RequestAuthority for FileRequestAuthority {
    fn authorized(&self) -> bool {
        *self.actor.borrow()
            && self.actor.has_changed().is_ok()
            && self.root.authorized()
            && self.clock.monotonic_ms() < self.deadline
            && self.keep.as_ref().is_none_or(|work| work.run_is_live())
            && self
                .outer
                .as_ref()
                .is_none_or(|cancel| !cancel.is_cancelled())
    }
    async fn revoked(&self) {
        let mut outer = self.outer.clone();
        let mut actor = self.actor.clone();
        while self.authorized() {
            let remaining = self.deadline.saturating_sub(self.clock.monotonic_ms());
            tokio::select! {
                _ = self.clock.sleep(Duration::from_millis(remaining)) => {},
                _ = actor.changed() => {},
                _ = self.root.revoked() => return,
                _ = async { if let Some(work) = &self.keep { work.retired().await } else { std::future::pending::<()>().await } } => return,
                _ = async { if let Some(cancel) = &mut outer { cancel.cancelled().await } else { std::future::pending::<()>().await } } => return,
            }
        }
    }
}

/// Publish cancellation explicitly during abort/unwind, including while other sender clones
/// still exist. Closed publishers now also fail closed at the transport admission boundary.
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
    Download(oneshot::Sender<Result<DownloadReady, String>>),
    AutomaticKeep,
}

impl ReadReply {
    fn is_closed(&self) -> bool {
        match self {
            Self::Chunk(reply) => reply.is_closed(),
            Self::Range(reply) => reply.is_closed(),
            Self::Keep(reply) => reply.is_closed(),
            Self::Download(reply) => reply.is_closed(),
            Self::AutomaticKeep => false,
        }
    }

    async fn closed(&mut self) {
        match self {
            Self::Chunk(reply) => reply.closed().await,
            Self::Range(reply) => reply.closed().await,
            Self::Keep(reply) => reply.closed().await,
            Self::Download(reply) => reply.closed().await,
            Self::AutomaticKeep => std::future::pending().await,
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
            Self::Download(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::AutomaticKeep => {}
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
    token: SavedRun,
    manifest: crate::FileManifest,
    hasher: catcoms_storage::CidHasher,
    plan_version: [u8; 32],
    /// A whole-candidate turn can visit each original plan at most once. Restart preserves the
    /// durable global allowance and bad-reference verdicts, not this ephemeral search cursor.
    tried: Vec<[u8; 32]>,
}

#[derive(Clone)]
enum SavedRequest {
    Keep {
        resume_existing: bool,
    },
    Download {
        resume: Option<catcoms_storage::kept::DownloadId>,
        /// A "Save as" choice remembered in the record right after its run starts.
        planned: Option<Arc<catcoms_storage::kept::ChosenDestination>>,
    },
}

#[derive(Clone)]
enum ReadCandidate {
    Direct(FileRef, PeerId),
    Bridge(FileRef, BridgeChoice),
}
impl ReadCandidate {
    fn reference(&self) -> &FileRef {
        match self {
            Self::Direct(reference, _) | Self::Bridge(reference, _) => reference,
        }
    }
    fn peer(&self) -> PeerId {
        match self {
            Self::Direct(_, peer) => *peer,
            Self::Bridge(_, choice) => choice.helper_peer(),
        }
    }
    fn into_parts(self) -> (FileRef, PeerId, Option<BridgeChoice>) {
        match self {
            Self::Direct(reference, peer) => (reference, peer, None),
            Self::Bridge(reference, choice) => (reference, choice.helper_peer(), Some(choice)),
        }
    }
    fn from_parts(reference: FileRef, peer: PeerId, choice: Option<BridgeChoice>) -> Self {
        match choice {
            Some(choice) => Self::Bridge(reference, choice),
            None => Self::Direct(reference, peer),
        }
    }
}
enum PreparedRead<T: MeshTransport> {
    Direct(catcoms_sync::PendingBlobFetch<T>),
    Bridge(PreparedBridgePage),
}

pub(super) struct ReadJob {
    cid: Cid,
    version: [u8; 32],
    epoch: u64,
    index: usize,
    deadline: u64,
    cancel: Option<RequestCancellation>,
    /// Exact actor generation, including local steps and already-returned network completions.
    actor: watch::Receiver<bool>,
    /// Original Root/member/policy scope, never refreshed when an old job retries or resumes.
    root: Arc<dyn catcoms_rt::RequestAuthority>,
    reply: ReadReply,
    range: Option<RangeState>,
    keep: Option<KeepState>,
    candidates: Option<VecDeque<ReadCandidate>>,
    /// Per-chunk helper/source coverage remains separate from direct-provider batches. It
    /// captures this original native read, not whichever account happens to be current later.
    bridge: Option<BridgeCatalog>,
    bridge_retries: u8,
    /// Per-chunk/per-round search cursor. At most MAX_CONNECTED_PEER_SNAPSHOT distinct peers;
    /// busy entries stay in candidates, while completed batches make room for later holders.
    tried_providers: HashSet<PeerId>,
    /// An exact authenticated reference which failed opening cannot be repaired by another
    /// holder. A different wrapped key can still open the same ciphertext CID, so the verdict
    /// is keyed by the full FileRef encoding across batches/retry rounds for this chunk.
    rejected_variants: HashSet<Vec<u8>>,
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
    // LAST: staged/range/search and reply metadata must retire before returning job capacity.
    _job: JobCustody,
}

impl ReadJob {
    fn current<T: MeshTransport, R: CryptoRngCore>(&self, server: &Server<T, R>) -> bool {
        *self.actor.borrow()
            && self.actor.has_changed().is_ok()
            && self.root.authorized()
            && !self.reply.is_closed()
            && !self
                .cancel
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
            && server.epoch() == self.epoch
            && if let Some(keep) = &self.keep {
                keep.token.current(server, &self.cid, &keep.plan_version)
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
            let _ = keep.token.pause(server);
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
                self.bridge = None;
                self.bridge_retries = 0;
                self.tried_providers.clear();
                self.rejected_variants.clear();
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
                let _ = keep.token.block(server);
                self.fail(
                    server,
                    "kept file failed whole-file content verification".into(),
                );
                return None;
            }
            let result = keep.token.finish(server).map_err(|e| e.to_string());
            if result.is_err() {
                let _ = keep.token.pause(server);
            }
            match self.reply {
                ReadReply::Keep(reply) => {
                    let _ = reply.send(result.map(|_| ()));
                }
                ReadReply::Download(reply) => {
                    let _ = reply.send(result.and_then(|lease| {
                        let lease =
                            lease.ok_or_else(|| "Download ownership changed.".to_string())?;
                        let planned = server
                            .sync
                            .download_planned_destination(lease.id())
                            .map_err(|e| e.to_string())?
                            .map(|planned| planned.map(Arc::new));
                        Ok(DownloadReady {
                            lease,
                            manifest: keep.manifest.clone(),
                            root: self.root,
                            actor: self.actor,
                            export_custody: None,
                            planned,
                        })
                    }));
                }
                _ => {}
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
                self.bridge = None;
                self.bridge_retries = 0;
                self.tried_providers.clear();
                self.rejected_variants.clear();
                self.rounds = 0;
                self.retryable = false;
                self.staged = Vec::new();
                self.staged_cid = None;
                self.pages = 0;
                // Only accepted, verified chunk progress renews a crossing range's quiet
                // deadline. A final page discarded by the route fence never reaches this edge.
                self.deadline = server
                    .runtime_clock()
                    .monotonic_ms()
                    .saturating_add(READ_MS);
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
    /// Retained separately from the advancing catalog: lost replies retry this SAME source
    /// batch with a fresh signed grant and another durable debit, never skip the first holder.
    bridge_choice: Option<BridgeChoice>,
    result: Result<CompletedBlobFetch, String>,
    /// Original prepaid execution slice. A new actor turn cannot renew it to accept an old
    /// network result after pause, Retry, replacement or its original monotonic deadline.
    keep_work: Option<catcoms_storage::kept::KeepWork>,
    // A timeout response may be queued while the actual request also remains in libp2p. Holding
    // this token in BOTH places bounds each ownership phase without prematurely refunding either.
    _accounting: SharedRequestKeepalive,
}

pub(super) struct FileTransfers {
    /// Sole publisher: clones/handles/lower workers only observe this generation. Drop retires
    /// it even if a transport continues to own the original request/accounting.
    lifetime: watch::Sender<bool>,
    running: JoinSet<TransferStep>,
    jobs: Arc<Semaphore>,
    process_jobs: Arc<Semaphore>,
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
    automatic_due: Option<u64>,
    automatic_after: Option<Cid>,
    /// Test-only causal boundary after real AEAD opening and before storage admission. No
    /// production callback runs here; tests retire the original owner at this exact boundary.
    #[cfg(test)]
    before_storage_admission: Option<Box<dyn FnOnce() + Send>>,
}

/// Observes the SAME actor generation as outgoing reads. Neither a source task nor a native
/// service attachment can keep the publisher alive or revive work after a cancelled freeze.
#[derive(Debug)]
struct ActorFileAuthority(watch::Receiver<bool>);
#[async_trait::async_trait]
impl catcoms_rt::RequestAuthority for ActorFileAuthority {
    fn authorized(&self) -> bool {
        *self.0.borrow() && self.0.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut live = self.0.clone();
        while self.authorized() {
            if live.changed().await.is_err() {
                return;
            }
        }
    }
}

impl TransferStep {
    pub(super) fn saved_progress(&self) -> Option<SavedProgress> {
        let job = match self {
            Self::Local(job) => job.as_ref(),
            Self::Network(finished) => &finished.job,
        };
        job.keep.as_ref().map(|keep| SavedProgress {
            run: keep.token,
            cid: job.cid,
        })
    }
}

/// Owner-specific, cheap observations around one actor completion. A same-CID Keep cannot
/// hide private Download progress or cause the UI to miss its terminal/paused transition.
pub(super) struct SavedProgress {
    run: SavedRun,
    cid: Cid,
}
impl SavedProgress {
    pub(super) fn observe<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
    ) -> Option<catcoms_storage::kept::KeepProgress> {
        match self.run {
            SavedRun::Keep(_) => server.sync.keep_progress(&self.cid),
            SavedRun::Download(run) => server.sync.download_progress(run.id()),
        }
    }
}

impl FileTransfers {
    pub(super) fn authority(&self) -> Arc<dyn catcoms_rt::RequestAuthority> {
        Arc::new(ActorFileAuthority(self.lifetime.subscribe()))
    }

    pub(super) fn new() -> Self {
        Self {
            lifetime: watch::channel(true).0,
            running: JoinSet::new(),
            jobs: Arc::new(Semaphore::new(SERVER_ATTEMPTS)),
            process_jobs: process_job_slots(),
            slots: Arc::new(Semaphore::new(SERVER_ATTEMPTS)),
            process: process_slots(),
            inflight: HashSet::new(),
            automatic_due: None,
            automatic_after: None,
            #[cfg(test)]
            before_storage_admission: None,
        }
    }

    fn admit_job(&self) -> Option<JobCustody> {
        self.jobs
            .clone()
            .try_acquire_owned()
            .ok()
            .zip(self.process_jobs.clone().try_acquire_owned().ok())
            .map(|(server, process)| JobCustody {
                _server: server,
                _process: process,
            })
    }

    fn before_storage_admission(&mut self) {
        #[cfg(test)]
        if let Some(hook) = self.before_storage_admission.take() {
            hook();
        }
    }

    /// Retire synchronously BEFORE acknowledging stop or awaiting the close event consumer.
    /// Do not drop/refund actual lower work here: captured observers invalidate its next effect,
    /// while transport-held AttemptAccounting continues to own every real outstanding permit.
    pub(super) fn retire(&self) {
        self.lifetime.send_replace(false);
    }

    /// A cancelled freeze may resume this actor, but it must never revive old queued work.
    /// Old jobs/completions retain the retired receiver; only new jobs capture this fresh owner.
    pub(super) fn resume(&mut self) {
        if !*self.lifetime.borrow() {
            self.lifetime = watch::channel(true).0;
        }
    }

    /// Revisit saved user intents independently of frontend views. The exact native session
    /// token follows every lower request; a fresh unlock cannot revive an old queued run.
    /// Cursor rotation includes unsuccessful choices so an absent first file cannot monopolize
    /// this bounded scheduler. Expired clocks disable it rather than leaving a ready loop.
    pub(super) fn pump_automatic<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        session: Option<RequestCancellation>,
    ) -> Option<u64> {
        let Some(session) = session.filter(|session| !session.is_cancelled()) else {
            self.automatic_due = None;
            return None;
        };
        let Some(cid) = server.sync.automatic_keep_candidate(self.automatic_after) else {
            self.automatic_due = None;
            return None;
        };
        let now = server.runtime_clock().monotonic_ms();
        let next = now.checked_add(AUTOMATIC_KEEP_MS)?;
        let due = *self.automatic_due.get_or_insert(next);
        if now < due {
            return Some(due);
        }
        self.automatic_due = Some(next);
        self.automatic_after = Some(cid);
        if server.sync.keep_auto_source_available(&cid) {
            // Admission returns immediately; the local prefix and network response are still
            // processed by the existing yielding/verification path with the original budget.
            let _ = self.start_keep(
                server,
                cid.as_bytes().to_vec(),
                Some(session),
                ReadReply::AutomaticKeep,
                true,
            );
        }
        Some(next)
    }

    pub(super) fn request_keep<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        raw: Vec<u8>,
        session: Option<RequestCancellation>,
    ) -> Result<(), String> {
        let session = session
            .filter(|session| !session.is_cancelled())
            .ok_or("the vault is locked")?;
        self.start_keep(server, raw, Some(session), ReadReply::AutomaticKeep, false)
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
        let _ = self.start_keep(server, raw, cancel, ReadReply::Keep(reply), false);
    }

    fn start_keep<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        raw: Vec<u8>,
        cancel: Option<RequestCancellation>,
        reply: ReadReply,
        resume_existing: bool,
    ) -> Result<(), String> {
        self.start_saved(
            server,
            raw,
            cancel,
            reply,
            SavedRequest::Keep { resume_existing },
        )
    }

    /// Observe a saved Download for the Resume preflight (review U-M3). Captures the CURRENT actor
    /// lifetime and Root authority, so a freeze/thaw or Root change after this call retires every
    /// edge and recovery effect made from it. The plan comes from the sealed saved manifest; it is
    /// used only to verify an already linked external file read-only.
    pub(super) fn download_preflight<T: MeshTransport + 'static, R: CryptoRngCore>(
        &self,
        server: &mut Server<T, R>,
        id: catcoms_storage::kept::DownloadId,
        cancel: &RequestCancellation,
    ) -> Result<super::DownloadPreflight, String> {
        if cancel.is_cancelled() {
            return Err("The download is no longer active.".into());
        }
        let root = server
            .sync
            .file_effect_authority()
            .map_err(|e| e.to_string())?;
        let (observation, state) = server
            .sync
            .download_observation(id)
            .map_err(|e| e.to_string())?;
        // Equivalent encrypted variants share one plaintext layout, so the first saved plan's
        // manifest describes the file; one that does not decode simply yields no plan.
        let plan = server
            .sync
            .download_saved_plans(id)
            .ok()
            .and_then(|plans| plans.into_iter().next())
            .and_then(|plan| kept_plan(plan.manifest, plan.cid).ok())
            .and_then(|(_, manifest)| {
                catcoms_storage::kept::ExportPlan::from_manifest(&manifest).ok()
            });
        Ok(super::DownloadPreflight {
            observation,
            state,
            plan,
            root,
            actor: self.lifetime.subscribe(),
        })
    }

    pub(super) fn download<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        raw: Vec<u8>,
        resume: Option<catcoms_storage::kept::DownloadId>,
        planned: Option<Arc<catcoms_storage::kept::ChosenDestination>>,
        cancel: RequestCancellation,
        reply: oneshot::Sender<Result<DownloadReady, String>>,
    ) {
        let _ = self.start_saved(
            server,
            raw,
            Some(cancel),
            ReadReply::Download(reply),
            SavedRequest::Download { resume, planned },
        );
    }

    fn start_saved<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        raw: Vec<u8>,
        cancel: Option<RequestCancellation>,
        reply: ReadReply,
        request: SavedRequest,
    ) -> Result<(), String> {
        let root = match server.sync.file_effect_authority() {
            Ok(root) => root,
            Err(error) => {
                let error = error.to_string();
                reply.fail(error.clone());
                return Err(error);
            }
        };
        if reply.is_closed() {
            return Err("file keeping cancelled".into());
        }
        if cancel
            .as_ref()
            .is_some_and(RequestCancellation::is_cancelled)
        {
            reply.fail("file keeping cancelled".into());
            return Err("file keeping cancelled".into());
        }
        if !server.sync.has_file_key() {
            // Legacy absent-transfer state can recover valid metadata without its file key.
            // Refuse before reservation/debit; this is not evidence the saved content is bad.
            let error = catcoms_sync::SyncError::FileKeyUnavailable.to_string();
            reply.fail(error.clone());
            return Err(error);
        }
        // Refuse BEFORE starting/resuming a durable run or constructing its retained state.
        // Busy is not a cancellation of an existing saved intent and creates no journal debit.
        let Some(job_custody) = self.admit_job() else {
            let error = "file transfers are busy; try again shortly".to_string();
            reply.fail(error.clone());
            return Err(error);
        };
        let result = (|| {
            let cid = Cid::from_bytes(
                raw.as_slice()
                    .try_into()
                    .map_err(|_| "bad content address".to_string())?,
            );
            let download = matches!(request, SavedRequest::Download { .. });
            let saved_download = if let SavedRequest::Download { resume, planned } = &request {
                let resume = *resume;
                let found = server
                    .sync
                    .downloads()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|d| resume.map_or(d.state.plan.cid == cid, |id| d.id == id));
                if resume.is_some() && found.is_none() {
                    return Err("Saved download changed.".into());
                }
                if found.as_ref().is_some_and(|d| d.state.plan.cid != cid) {
                    return Err("Saved download changed.".into());
                }
                // Picker review L2: a "Save as" choice is refused while a journal exists (a
                // chosen place never redirects an existing partial). Refuse it HERE, before
                // `resume_download` retires the record's read epoch: otherwise a Save as from
                // the chat would interrupt a live journaled save of the same file and only then
                // be refused by `plan_download_destination`. Same text as that storage refusal.
                let journaled = found
                    .as_ref()
                    .is_some_and(|d| !matches!(d.export, catcoms_storage::kept::ExportView::None));
                if journaled && planned.is_some() {
                    return Err(SAVE_AS_OVER_JOURNAL.into());
                }
                found
            } else {
                None
            };
            let inventory = server.sync.kept_files();
            let existing = if download {
                saved_download
                    .as_ref()
                    .map(|d| (d.state.plan.clone(), d.state.listing))
            } else {
                inventory
                    .transfers
                    .into_iter()
                    .filter(|transfer| transfer.phase != catcoms_storage::kept::KeepPhase::Deleting)
                    .map(|transfer| (transfer.plan, transfer.listing))
                    .chain(inventory.files.into_iter().map(|file| {
                        let listing = file.plan.version;
                        (file.plan, listing)
                    }))
                    .find(|(plan, _)| plan.cid == cid)
            };
            // A user may explicitly check/repair their own saved manifest even after unlisting.
            // This local authority never enters files(), media heads, or replicated publication.
            let (plan, manifest, listing, candidates) = if let Some((plan, listing)) = existing {
                let (validated, manifest) = kept_plan(plan.manifest.clone(), cid)?;
                if validated != plan {
                    return Err("kept manifest is inconsistent".into());
                }
                let saved = if let Some(d) = &saved_download {
                    server
                        .sync
                        .download_saved_plans(d.id)
                        .map_err(|e| e.to_string())?
                } else {
                    server
                        .sync
                        .kept_saved_plans(&cid)
                        .ok_or("saved transfer changed")?
                };
                if saved.is_empty() || saved.len() > file_resolution::MAX_MANIFEST_VARIANTS {
                    return Err("Saved file details could not be verified.".into());
                }
                for (index, candidate) in saved.iter().enumerate() {
                    let (validated, decoded) = kept_plan(candidate.manifest.clone(), cid)?;
                    if &validated != candidate
                        || !file_resolution::equivalent(&manifest, &decoded)
                        || (index != 0 && saved[index - 1].version >= candidate.version)
                    {
                        return Err("Saved file details could not be verified.".into());
                    }
                }
                // Old exact-only records may have chosen one member of an earlier set. They
                // cannot reconstruct it after unlisting. V4 retained every candidate, so require
                // the exact original set identity before spending any execution allowance.
                if !saved.contains(&plan)
                    || (saved.len() > 1
                        && file_resolution::manifest_set_version(saved.iter().map(|p| p.version))
                            != listing)
                {
                    return Err("Saved file details could not be verified.".into());
                }
                (plan, manifest, listing, None)
            } else {
                if matches!(
                    request,
                    SavedRequest::Keep {
                        resume_existing: true
                    }
                ) {
                    return Err("saved transfer changed".into());
                }
                let resolved = file_resolution::resolve(&server.files(), &cid)
                    .ok_or("no unambiguous file manifest in this server's index")?;
                let plans = resolved
                    .variants
                    .iter()
                    .map(|(entry, _)| kept_plan(entry.file_ref.clone(), cid))
                    .collect::<Result<Vec<_>, _>>()?;
                let chosen = plans
                    .iter()
                    .position(|(_, manifest)| {
                        manifest
                            .chunks
                            .iter()
                            .all(|r| server.sync.has_blob(&r.ciphertext_cid))
                    })
                    .unwrap_or(0);
                let (plan, manifest) = plans[chosen].clone();
                let candidates = (download || plans.len() > 1)
                    .then(|| (plans.into_iter().map(|(p, _)| p).collect(), chosen));
                (plan, manifest, resolved.version, candidates)
            };
            let plan_version = plan.version;
            // Whether this step creates the Download record (no saved record was found). Only
            // such a record may be released outright when its Save as choice cannot be written.
            let created = download && saved_download.is_none();
            let token = if let Some(d) = saved_download {
                SavedRun::Download(
                    server
                        .sync
                        .resume_download(d.id)
                        .map_err(|e| e.to_string())?,
                )
            } else if let Some((plans, selected)) = candidates {
                if download {
                    SavedRun::Download(
                        server
                            .sync
                            .begin_download(plans, selected, listing)
                            .map_err(|e| e.to_string())?,
                    )
                } else {
                    SavedRun::Keep(
                        server
                            .sync
                            .begin_keep_variants(plans, selected, listing)
                            .map_err(|e| e.to_string())?,
                    )
                }
            } else {
                SavedRun::Keep(
                    server
                        .sync
                        .begin_keep_with_listing(plan, listing)
                        .map_err(|e| e.to_string())?,
                )
            };
            // Remember a "Save as" choice in the same synchronous step, before any network
            // work: an interrupted acquisition, Pause, Lock or restart then still saves there.
            // If that write is refused, undo the run we just started so nothing is left holding
            // the single active-run slot (and, picker review L2, no "Interrupted" row is left
            // for a brand-new record that never got its choice).
            if let (
                SavedRun::Download(run),
                SavedRequest::Download {
                    planned: Some(choice),
                    ..
                },
            ) = (&token, &request)
            {
                if let Err(error) = server.sync.plan_download_destination(*run, choice) {
                    abandon_unplanned(server, token, created);
                    return Err(error.to_string());
                }
            }
            Ok::<_, String>((cid, listing, token, manifest, plan_version))
        })();
        match result {
            Ok((cid, version, token, manifest, plan_version)) => {
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
                    actor: self.lifetime.subscribe(),
                    root,
                    reply,
                    range: None,
                    keep: Some(KeepState {
                        token,
                        manifest,
                        hasher: catcoms_storage::CidHasher::new(),
                        plan_version,
                        tried: vec![plan_version],
                    }),
                    candidates: None,
                    bridge: None,
                    bridge_retries: 0,
                    tried_providers: HashSet::new(),
                    rejected_variants: HashSet::new(),
                    error: "no connected provider supplied the kept file's exact chunk".into(),
                    rounds: 0,
                    retryable: false,
                    staged: Vec::new(),
                    staged_cid: None,
                    page_size: catcoms_sync::MIN_BLOB_PAGE,
                    pages: 0,
                    paging: true,
                    _job: job_custody,
                };
                self.yield_local(job);
                Ok(())
            }
            Err(error) => {
                reply.fail(error.clone());
                Err(error)
            }
        }
    }

    /// A new candidate restarts the ENTIRE plaintext hash and exact chunk sequence. The saved
    /// reservation, grant, intent and cancellation are unchanged; only storage can mint its new
    /// opaque run after retiring the old lower-effect owner and publishing the selection.
    fn fallback_or_fail<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        mut job: ReadJob,
        bad_current: bool,
        error: String,
    ) {
        let outcome = (|| -> Result<bool, String> {
            let Some(keep) = job.keep.as_mut() else {
                return Ok(false);
            };
            let Some((token, plan)) = keep
                .token
                .fallback(server, &keep.plan_version, &keep.tried, bad_current)
                .map_err(|e| e.to_string())?
            else {
                return Ok(false);
            };
            // Install the new token first, so even a corrupt saved plan's rejection pauses its
            // actual owner. Full saved-set validation already ran before admission and plans are immutable.
            keep.token = token;
            let (validated, manifest) = kept_plan(plan.manifest.clone(), job.cid)?;
            if plan != validated || !file_resolution::equivalent(&keep.manifest, &manifest) {
                return Err("Saved file details could not be verified.".into());
            }
            keep.plan_version = plan.version;
            keep.tried.push(plan.version);
            keep.manifest = manifest;
            keep.hasher = catcoms_storage::CidHasher::new();
            job.index = 0;
            job.candidates = None;
            job.bridge = None;
            job.bridge_retries = 0;
            job.tried_providers.clear();
            job.rejected_variants.clear();
            job.staged = Vec::new();
            job.staged_cid = None;
            job.page_size = catcoms_sync::MIN_BLOB_PAGE;
            job.pages = 0;
            job.paging = true;
            job.rounds = 0;
            job.retryable = false;
            job.deadline = server
                .runtime_clock()
                .monotonic_ms()
                .saturating_add(READ_MS);
            Ok(true)
        })();
        match outcome {
            Ok(true) => self.yield_local(job),
            Ok(false) => {
                if bad_current {
                    if let Some(keep) = &job.keep {
                        let _ = keep.token.block(server);
                    }
                }
                job.fail(server, error);
            }
            Err(error) => job.fail(server, error),
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn range_cancellable<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        cid: Vec<u8>,
        version: [u8; 32],
        start: u64,
        max_len: usize,
        cancel: Option<RequestCancellation>,
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
            cancel,
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
        if cancel
            .as_ref()
            .is_some_and(RequestCancellation::is_cancelled)
        {
            reply.fail("file download cancelled".into());
            return;
        }
        let Ok(raw) = <[u8; 32]>::try_from(raw.as_slice()) else {
            reply.fail("bad content address".into());
            return;
        };
        let cid = Cid::from_bytes(raw);
        let Some(job_custody) = self.admit_job() else {
            reply.fail("file transfers are busy; try again shortly".into());
            return;
        };
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
        let root = match server.sync.file_effect_authority() {
            Ok(root) => root,
            Err(error) => {
                reply.fail(error.to_string());
                return;
            }
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
            actor: self.lifetime.subscribe(),
            root,
            reply,
            range,
            keep: None,
            candidates: None,
            bridge: None,
            bridge_retries: 0,
            tried_providers: HashSet::new(),
            rejected_variants: HashSet::new(),
            error: format!("file not available yet; no connected provider supplied chunk {index}"),
            rounds: 0,
            retryable: false,
            staged: Vec::new(),
            staged_cid: None,
            page_size: catcoms_sync::MIN_BLOB_PAGE,
            pages: 0,
            paging: true,
            _job: job_custody,
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
                self.fallback_or_fail(server, job, false, "file download timed out".into());
                return;
            }
            let local_work = if let Some(keep) = &job.keep {
                match keep.token.work(server, None) {
                    Ok(work) => Some(work),
                    Err(error) => {
                        job.fail(server, error.to_string());
                        return;
                    }
                }
            } else {
                None
            };
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
                    if job.rejected_variants.contains(&reference.encode()) {
                        continue;
                    }
                    let retained = job.keep.as_ref().and_then(|keep| {
                        keep.token
                            .get(
                                server,
                                &reference.ciphertext_cid,
                                catcoms_sync::MAX_BOUNDED_BLOB_BYTES,
                            )
                            .ok()
                            .flatten()
                    });
                    if let Some(ciphertext) =
                        retained.or_else(|| server.sync.get_blob(&reference.ciphertext_cid))
                    {
                        let opened = server.sync.open_file(&ciphertext, reference);
                        self.before_storage_admission();
                        // Failed AEAD is also a durable effect when it rejects a Keep candidate.
                        // Check the original invocation after CPU work on BOTH result paths.
                        if let Some(work) = &local_work {
                            if let Err(error) = server.sync.validate_keep_work(work) {
                                job.fail(server, error.to_string());
                                return;
                            }
                        }
                        // Work validation samples the injected Clock. No later Clock/native
                        // callback may sit between this final owner check and storage/verdict.
                        if !job.current(server) {
                            job.fail(server, "file download cancelled".into());
                            return;
                        }
                        if let Ok(bytes) = opened {
                            if let Some(keep) = &job.keep {
                                if let Err(error) = keep.token.put(server, &ciphertext) {
                                    job.fail(server, error.to_string());
                                    return;
                                }
                            }
                            held = Some(bytes);
                            break;
                        }
                        job.error = format!("chunk {} could not be decrypted", job.index);
                        if job.keep.is_some() {
                            let error = job.error.clone();
                            self.fallback_or_fail(server, job, true, error);
                            return;
                        }
                        job.rejected_variants.insert(reference.encode());
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
                let peers = server.sync.blob_fetch_peers_excluding(&job.tried_providers);
                job.tried_providers.extend(peers.iter().copied());
                job.candidates = Some(
                    missing
                        .into_iter()
                        .flat_map(|reference| {
                            peers
                                .iter()
                                .map(move |peer| ReadCandidate::Direct(reference.clone(), *peer))
                        })
                        .collect(),
                );
            }
            // Take the first candidate whose provider is not already carrying a bulk request,
            // rotating the busy ones to the back rather than discarding them: a peer that is
            // busy now is a perfectly good provider a moment later, and dropping it here would
            // reintroduce the single-attempt failure through the other door.
            let mut picked = None;
            let mut deferred = false;
            if let Some(candidates) = job.candidates.as_mut() {
                for _ in 0..candidates.len() {
                    let Some(candidate) = candidates.pop_front() else {
                        break;
                    };
                    // Retry the same sources when the authenticated helper changes PeerId.
                    // Refresh before checking old peer connectivity; old scopes never revive.
                    let candidate = match candidate {
                        ReadCandidate::Bridge(reference, choice) => {
                            let Some(catalog) = job.bridge.as_mut() else {
                                continue;
                            };
                            match server.sync.refresh_file_bridge_choice(catalog, &choice) {
                                Ok(choice) => ReadCandidate::Bridge(reference, choice),
                                Err(BridgeError::Unavailable) => {
                                    // Relinquish this old route without parking it ahead of a
                                    // usable D. The catalog retains its bounded later sweep.
                                    server.sync.file_bridge_unavailable(catalog, &choice);
                                    continue;
                                }
                                Err(_) => continue,
                            }
                        }
                        direct => direct,
                    };
                    let peer = candidate.peer();
                    // A proven or bootstrap candidate can disconnect during another attempt. Drop
                    // it without spending a permit or creating a request that could redial from
                    // remembered routes.
                    if !server.sync.blob_peer_is_connected(peer) {
                        if let ReadCandidate::Bridge(_, choice) = &candidate {
                            if let Some(catalog) = job.bridge.as_mut() {
                                server.sync.file_bridge_unavailable(catalog, choice);
                            }
                        }
                        continue;
                    }
                    if self.inflight.contains(&peer) {
                        candidates.push_back(candidate);
                        deferred = true;
                        continue;
                    }
                    picked = Some(candidate);
                    break;
                }
            }
            // Exhausted direct candidates do not establish that no indirect source exists.
            if picked.is_none() {
                let peers = server.sync.blob_fetch_peers_excluding(&job.tried_providers);
                if !peers.is_empty() {
                    // A busy preferred provider must not hide later holders. Keep its queued
                    // entry while appending a bounded new batch; park only after the complete
                    // search has no usable free provider. At most 320 peers x 4 exact variants
                    // are retained, independent of proof reorderings or connection churn.
                    let references: Vec<_> = if let Some(keep) = &job.keep {
                        keep.manifest
                            .chunks
                            .get(job.index)
                            .cloned()
                            .into_iter()
                            .collect()
                    } else {
                        file_resolution::resolve(&server.files(), &job.cid)
                            .into_iter()
                            .flat_map(|resolved| resolved.variants)
                            .filter_map(|(_, manifest)| manifest.chunks.get(job.index).cloned())
                            .collect()
                    };
                    job.tried_providers.extend(peers.iter().copied());
                    let candidates = job
                        .candidates
                        .as_mut()
                        .expect("provider search initialized");
                    for reference in references {
                        if !job.rejected_variants.contains(&reference.encode()) {
                            candidates.extend(
                                peers
                                    .iter()
                                    .map(|peer| ReadCandidate::Direct(reference.clone(), *peer)),
                            );
                        }
                    }
                    continue;
                }
                if !deferred && job.bridge.is_none() {
                    if let Some(origin) = job.cancel.clone() {
                        job.bridge = server
                            .sync
                            .begin_file_bridge_catalog(origin, job.root.clone())
                            .ok();
                    }
                }
                if let Some(catalog) = job.bridge.as_mut().filter(|_| !deferred) {
                    let references: Vec<_> = if let Some(keep) = &job.keep {
                        keep.manifest
                            .chunks
                            .get(job.index)
                            .cloned()
                            .into_iter()
                            .collect()
                    } else {
                        file_resolution::resolve(&server.files(), &job.cid)
                            .into_iter()
                            .flat_map(|resolved| resolved.variants)
                            .filter_map(|(_, manifest)| manifest.chunks.get(job.index).cloned())
                            .collect()
                    };
                    let versions: Vec<_> = references
                        .iter()
                        .filter(|reference| !job.rejected_variants.contains(&reference.encode()))
                        .filter_map(|reference| bridge_reference_variant(reference).ok())
                        .collect();
                    match server.sync.next_file_bridge_choice(catalog, &versions) {
                        Ok(BridgeStep::Ready(choice)) => {
                            if let Some(reference) = references.into_iter().find(|reference| {
                                bridge_reference_variant(reference).ok() == Some(choice.variant())
                            }) {
                                let candidate = ReadCandidate::Bridge(reference, choice);
                                job.bridge_retries = 0;
                                if self.inflight.contains(&candidate.peer()) {
                                    job.candidates.as_mut().unwrap().push_back(candidate);
                                    deferred = true;
                                } else {
                                    picked = Some(candidate);
                                }
                            }
                        }
                        Ok(BridgeStep::Yield) => {
                            // Bound local catalog work as well as network work. Large/sparse
                            // rosters yield to commands without spinning while helpers recover.
                            self.delay_local(server, job, 50);
                            return;
                        }
                        Ok(BridgeStep::Backoff) => {
                            self.delay_local(server, job, ROUND_BACKOFF_MS);
                            return;
                        }
                        Ok(BridgeStep::Exhausted) | Err(_) => {}
                    }
                }
            }
            let Some(candidate) = picked else {
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
                    job.tried_providers.clear();
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
                self.fallback_or_fail(server, job, false, error);
                return;
            };
            let permits = self
                .slots
                .clone()
                .try_acquire_owned()
                .ok()
                .zip(self.process.clone().try_acquire_owned().ok());
            let Some((local, global)) = permits else {
                // This read already owns a bounded whole-job slot. Older canceled requests
                // may still hold every actual lower permit; local pressure is not a failed
                // provider attempt. Keep the exact candidate and original deadline/allowance,
                // then retry quietly after the actual lower owners have had time to retire.
                job.candidates
                    .as_mut()
                    .expect("selected candidate inventory")
                    .push_front(candidate);
                self.delay_local(server, job, BUSY_PEER_BACKOFF_MS);
                return;
            };
            let (reference, peer, bridge_choice) = candidate.into_parts();
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
            let paged = bridge_choice.is_some() || job.paging;
            let desired = if bridge_choice.is_some() {
                catcoms_sync::MIN_BLOB_PAGE
            } else {
                job.page_size.min(server.sync.blob_page_ceiling(peer))
            };
            let (request_limit, prepaid_bytes) = if job.keep.is_some() {
                match kept_request_window(&reference, paged, desired, job.staged.len()) {
                    Ok(window) => window,
                    Err(error) => {
                        job.fail(server, error);
                        return;
                    }
                }
            } else {
                (
                    if paged {
                        desired
                    } else {
                        catcoms_sync::MAX_BOUNDED_BLOB_BYTES
                    },
                    0,
                )
            };
            let prepared = if let Some(choice) = &bridge_choice {
                let offset = match u32::try_from(job.staged.len()) {
                    Ok(offset) => offset,
                    Err(_) => {
                        job.fail(server, "staged chunk exceeds its limit".into());
                        return;
                    }
                };
                match server.sync.prepare_file_bridge_page(
                    job.bridge.as_ref().unwrap(),
                    choice,
                    &reference,
                    offset,
                    request_limit,
                ) {
                    Ok(prepared) => Ok(PreparedRead::Bridge(prepared)),
                    Err(BridgeError::Busy) => {
                        job.candidates
                            .as_mut()
                            .unwrap()
                            .push_front(ReadCandidate::Bridge(reference, choice.clone()));
                        self.delay_local(server, job, BUSY_PEER_BACKOFF_MS);
                        return;
                    }
                    Err(BridgeError::Unavailable) => {
                        server
                            .sync
                            .file_bridge_unavailable(job.bridge.as_mut().unwrap(), choice);
                        // No transport exists yet. Preserve search progress while allowing
                        // another current helper before waiting for this connection to return.
                        continue;
                    }
                    Err(error) => Err(error.to_string()),
                }
            } else if paged {
                let offset = match u32::try_from(job.staged.len()) {
                    Ok(offset) => offset,
                    Err(_) => {
                        job.fail(server, "staged chunk exceeds the addressable window".into());
                        return;
                    }
                };
                server
                    .sync
                    .prepare_blob_page(peer, reference.ciphertext_cid, offset, request_limit)
                    .map(PreparedRead::Direct)
                    .map_err(|error| error.to_string())
            } else {
                server
                    .sync
                    .prepare_blob_fetch(peer, reference.ciphertext_cid, request_limit)
                    .map(PreparedRead::Direct)
                    .map_err(|error| error.to_string())
            };
            let request = match prepared {
                Ok(request) => request,
                Err(error) => {
                    job.error = error;
                    continue;
                }
            };
            let clock = server.runtime_clock();
            // Capacity and authenticated preparation already succeeded. Debit persistent work
            // before the first actual dispatch, keeping a proven local refusal free of spend.
            let keep_work = if let Some(keep) = &job.keep {
                match keep.token.work(
                    server,
                    Some(catcoms_storage::kept::KeepNetworkWork {
                        bytes: prepaid_bytes,
                        paged,
                    }),
                ) {
                    Ok(work) => Some(work),
                    Err(error) => {
                        job.fail(server, error.to_string());
                        return;
                    }
                }
            } else {
                None
            };
            let request = match request {
                PreparedRead::Direct(request) => request,
                PreparedRead::Bridge(prepared) => match server
                    .sync
                    .commit_file_bridge_page(job.bridge.as_mut().unwrap(), prepared)
                {
                    Ok(request) => request,
                    Err(BridgeError::Busy) => {
                        job.candidates
                            .as_mut()
                            .unwrap()
                            .push_front(ReadCandidate::Bridge(reference, bridge_choice.unwrap()));
                        // A durable debit already occurred; it remains spent. Waiting avoids
                        // repeatedly charging an unavailable local pacing/route position.
                        self.delay_local(server, job, BUSY_PEER_BACKOFF_MS);
                        return;
                    }
                    Err(BridgeError::Unavailable) => {
                        server.sync.file_bridge_unavailable(
                            job.bridge.as_mut().unwrap(),
                            bridge_choice.as_ref().unwrap(),
                        );
                        // This proven unstarted route refusal preserves the cursor, never
                        // refunds the completed durable debit, and does not hide a fresh D.
                        continue;
                    }
                    Err(error) => {
                        job.error = error.to_string();
                        continue;
                    }
                },
            };
            let execution_deadline = keep_work
                .as_ref()
                .map_or(job.deadline, |work| work.deadline().min(job.deadline));
            let deadline = execution_deadline
                .min(clock.monotonic_ms().saturating_add(ATTEMPT_MS))
                .min(request.effect_deadline().unwrap_or(u64::MAX));
            let authority = Arc::new(FileRequestAuthority {
                clock: clock.clone(),
                deadline,
                keep: keep_work.clone(),
                outer: job.cancel.clone(),
                actor: job.actor.clone(),
                root: job.root.clone(),
            });
            tracing::debug!(
                cid = ?job.cid,
                chunk = job.index,
                ?peer,
                round = job.rounds,
                budget_ms = deadline.saturating_sub(clock.monotonic_ms()),
                "requesting a file chunk from a provider"
            );
            self.inflight.insert(peer);
            // Construct the guard BEFORE spawn so aborting even an unpolled task sets the bit.
            let (signal, receiver) = watch::channel(false);
            let guard = CancelOnDrop(signal);
            let cancellation = RequestCancellation::new(receiver, Some(accounting.clone()))
                .with_authority(authority);
            self.running.spawn(async move {
                let _guard = guard;
                // First polling can occur after a queued actor step or a long executor stall.
                // Recheck the original absolute deadline before even invoking the transport.
                let duration = Duration::from_millis(deadline.saturating_sub(clock.monotonic_ms()));
                let result = fetch_chunk_or_cancel(job.cancel.clone(), async {
                    if cancellation.is_cancelled() {
                        return Err(ATTEMPT_EXPIRED_ERROR.into());
                    }
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
                    bridge_choice,
                    result,
                    keep_work,
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
            bridge_choice,
            result,
            keep_work,
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
        if let Some(work) = &keep_work {
            if let Err(error) = server.sync.validate_keep_work(work) {
                let expired = matches!(
                    error,
                    catcoms_sync::SyncError::Storage(
                        catcoms_storage::StorageError::KeepWorkExpired
                    )
                );
                drop(_accounting);
                if expired {
                    self.fallback_or_fail(server, job, false, error.to_string());
                } else {
                    job.fail(server, error.to_string());
                }
                return;
            }
        }
        // A ready result can wait behind other actor work. Its original read deadline still
        // applies at commit time, rather than only while the worker was suspended on the wire.
        if server.runtime_clock().monotonic_ms() >= job.deadline {
            // A later prepaid slice does not extend this candidate's original quiet deadline.
            // Retire it and try an owned alternative without importing the late reply; absence
            // at this boundary is not evidence of a bad wrapped key or corrupt manifest.
            drop(_accounting);
            self.fallback_or_fail(server, job, false, "file download timed out".into());
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
        let effect = result
            .as_ref()
            .ok()
            .and_then(CompletedBlobFetch::effect_scope);
        // Previously accepted pages belong to the immutable ciphertext, independently of a
        // later page's connection lease. A rejected new page may not discard that earned prefix
        // and force finite saved-transfer allowance to pay for it all again.
        let accepted_prefix = (job.staged.len(), job.pages, job.page_size);
        let outcome = match result {
            Err(error) => {
                job.retryable = attempt_expired;
                Err(error)
            }
            Ok(completed) if paged => {
                match server.sync.authenticate_blob_page(completed) {
                    Ok(catcoms_sync::BlobPageOutcome::Page(page)) => {
                        if job.keep.is_some()
                            && kept_ciphertext_bound(&reference)
                                .is_ok_and(|bound| page.total as usize > bound)
                        {
                            job.fail(server, "chunk exceeds its declared size".into());
                            return;
                        }
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
                    Ok(catcoms_sync::BlobPageOutcome::Unavailable) => {
                        if let (Some(catalog), Some(choice)) = (job.bridge.as_mut(), &bridge_choice)
                        {
                            server.sync.file_bridge_unavailable(catalog, choice);
                        }
                        // Cover fresh sources first. Sleeping on each offline batch can make
                        // a late online holder unreachable within the original quiet deadline.
                        job.error = "file helper is temporarily unavailable".into();
                        Ok(None)
                    }
                    Ok(catcoms_sync::BlobPageOutcome::RouteChanged) => {
                        if let (Some(catalog), Some(choice)) = (job.bridge.as_mut(), &bridge_choice)
                        {
                            server.sync.file_bridge_unavailable(catalog, choice);
                        }
                        // Payload-free local observation. Fresh helpers get their turn before
                        // a bounded later sweep of the disconnected helper's original sources.
                        Ok(None)
                    }
                    Ok(catcoms_sync::BlobPageOutcome::Unsupported) => {
                        if let Some(choice) = &bridge_choice {
                            if let Some(catalog) = job.bridge.as_mut() {
                                server.sync.file_bridge_unsupported(catalog, choice);
                            }
                            // An old helper never gains a legacy whole-file forwarding path.
                            Ok(None)
                        } else {
                            // A build that predates paging. Restart this chunk on the whole-blob
                            // grammar rather than reading its silence as absence.
                            job.paging = false;
                            job.staged = Vec::new();
                            job.staged_cid = None;
                            job.pages = 0;
                            resume = true;
                            Ok(None)
                        }
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
            // Only retained partial bytes count as progress. A complete page whose route
            // retires during AEAD must not renew the quiet deadline or reset retry accounting.
            let next_deadline = server
                .runtime_clock()
                .monotonic_ms()
                .saturating_add(READ_MS);
            if let Some(effect) = &effect {
                match server.sync.blob_effect_status(effect) {
                    catcoms_sync::BlobEffectStatus::Current => {}
                    catcoms_sync::BlobEffectStatus::Retired => {
                        job.fail(server, "file download cancelled".into());
                        return;
                    }
                    catcoms_sync::BlobEffectStatus::RouteChanged => {
                        job.staged.truncate(accepted_prefix.0);
                        job.pages = accepted_prefix.1;
                        job.page_size = accepted_prefix.2;
                        if let (Some(catalog), Some(choice)) = (job.bridge.as_mut(), &bridge_choice)
                        {
                            server.sync.file_bridge_unavailable(catalog, choice);
                        }
                        drop(_accounting);
                        self.yield_local(job);
                        return;
                    }
                }
            }
            job.deadline = next_deadline;
            job.bridge_retries = 0;
            // Re-offer the provider this run is already mid-way through. Continuity is worth
            // preferring: another holder would serve the same bytes, but this one has a warm
            // path and the offset is only meaningful because the blob is immutable.
            if let Some(candidates) = job.candidates.as_mut() {
                candidates.push_front(ReadCandidate::from_parts(reference, peer, bridge_choice));
            }
            drop(_accounting);
            self.advance(server, job);
            return;
        }
        match outcome {
            Ok(Some((ciphertext, provider))) => {
                let opened = server.sync.open_file(&ciphertext, &reference);
                self.before_storage_admission();
                // Rejecting a wrapped key changes durable selection just as storing good
                // ciphertext does. Neither outcome may escape the original effect scope.
                if let Some(work) = &keep_work {
                    if let Err(error) = server.sync.validate_keep_work(work) {
                        job.fail(server, error.to_string());
                        return;
                    }
                }
                if !job.current(server) {
                    job.fail(server, "file download cancelled".into());
                    return;
                }
                if let Some(effect) = &effect {
                    match server.sync.blob_effect_status(effect) {
                        catcoms_sync::BlobEffectStatus::Current => {}
                        catcoms_sync::BlobEffectStatus::Retired => {
                            job.fail(server, "file download cancelled".into());
                            return;
                        }
                        catcoms_sync::BlobEffectStatus::RouteChanged => {
                            job.staged = ciphertext;
                            job.staged.truncate(accepted_prefix.0);
                            job.pages = accepted_prefix.1;
                            job.page_size = accepted_prefix.2;
                            if let (Some(catalog), Some(choice)) =
                                (job.bridge.as_mut(), &bridge_choice)
                            {
                                server.sync.file_bridge_unavailable(catalog, choice);
                            }
                            // Route loss during CPU work grants neither a storage write nor a
                            // bad-key verdict. Keep the original read/budget and try fresh routes.
                            drop(_accounting);
                            self.yield_local(job);
                            return;
                        }
                    }
                }
                match opened {
                    Ok(bytes) => {
                        let stored = if let Some(keep) = &job.keep {
                            keep.token.put(server, &ciphertext)
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
                    Err(error) => {
                        job.error = error.to_string();
                        if job.keep.is_some() {
                            let error = job.error.clone();
                            drop(_accounting);
                            self.fallback_or_fail(server, job, true, error);
                            return;
                        }
                    }
                }
                // A second provider cannot repair the exact authenticated ciphertext/key pair.
                job.rejected_variants.insert(reference.encode());
                if let Some(candidates) = job.candidates.as_mut() {
                    candidates
                        .retain(|candidate| candidate.reference().encode() != reference.encode());
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
                if let Some(choice) = bridge_choice {
                    if job.retryable && job.bridge_retries < 3 {
                        job.bridge_retries += 1;
                        job.candidates
                            .as_mut()
                            .unwrap()
                            .push_front(ReadCandidate::Bridge(reference, choice));
                        job.error = error;
                        drop(_accounting);
                        self.delay_local(server, job, ROUND_BACKOFF_MS);
                        return;
                    }
                    if job.retryable {
                        if let Some(catalog) = job.bridge.as_mut() {
                            server.sync.file_bridge_unavailable(catalog, &choice);
                        }
                    }
                }
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
