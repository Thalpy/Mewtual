//! Bounded file reads whose network suspension never borrows the Server. The actor prepares and
//! commits each step; workers own only an opaque signed request, cancellation and reply lifetime.

use std::collections::VecDeque;
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
const ATTEMPT_MS: u64 = 8_000;
const READ_MS: u64 = 60_000;

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
    result: Result<CompletedBlobFetch, String>,
    // A timeout response may be queued while the actual request also remains in libp2p. Holding
    // this token in BOTH places bounds each ownership phase without prematurely refunding either.
    _accounting: SharedRequestKeepalive,
}

pub(super) struct FileTransfers {
    running: JoinSet<TransferStep>,
    slots: Arc<Semaphore>,
    process: Arc<Semaphore>,
}

impl FileTransfers {
    pub(super) fn new() -> Self {
        Self {
            running: JoinSet::new(),
            slots: Arc::new(Semaphore::new(SERVER_ATTEMPTS)),
            process: process_slots(),
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
            let Some((reference, peer)) = job.candidates.as_mut().and_then(VecDeque::pop_front)
            else {
                let error = job.error.clone();
                job.fail(server, error);
                return;
            };
            // A proven or bootstrap candidate can disconnect during another attempt. Skip it without
            // spending a permit or creating a request that could redial from remembered routes.
            if !server.sync.blob_fetch_peers().contains(&peer) {
                continue;
            }
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
            let request = match server.sync.prepare_blob_fetch(
                peer,
                reference.ciphertext_cid,
                catcoms_sync::MAX_BOUNDED_BLOB_BYTES,
            ) {
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
                        _ = clock.sleep(duration) => Err("file provider timed out".into()),
                        result = request.fetch(cancellation) => Ok(result),
                    }
                })
                .await;
                TransferStep::Network(Box::new(FinishedAttempt {
                    job,
                    reference,
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
            result,
            _accounting,
        } = finished;
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
        match result.and_then(|result| {
            server
                .sync
                .authenticate_blob_fetch(result)
                .map_err(|error| error.to_string())
        }) {
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
            Ok(None) => {}
            Err(error) => job.error = error,
        }
        // Drop this completed-result share before retry admission. A genuinely still-live lower
        // request retains its own share; it cannot be refunded by this transition.
        drop(_accounting);
        self.advance(server, job);
    }
}

#[cfg(test)]
mod tests;
