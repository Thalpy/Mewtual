//! Affine custody for the full blocking source read, independently of small network page limits.
//! A timed-out actor wait cannot cancel a blocking closure or recycle its source allowance.
use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use catcoms_rt::{RequestAuthority, SharedRequestKeepalive};
use catcoms_storage::{BlobReadPlan, BlobReadResult, BlobStore, Cid, StorageError};
use thiserror::Error;

/// New indirect service handles current bounded file chunks, not legacy whole 16 MiB blobs.
pub const MAX_SOURCE_CHUNK: usize = 8 * 1024 * 1024 + 44;
/// Explicit scratch/metadata capacity, not allocator, OS or unrelated process overhead. The
/// storage derivation for MAX_SOURCE_CHUNK exceeds 32 MiB and must never be rounded down.
pub const SOURCE_READ_BYTES: usize = 40 * 1024 * 1024;
const PROCESS_READS: usize = 2;
const MAX_GROUP_BYTES: usize = 256;

/// Admission, blocking storage and request-lifecycle failures. A capacity refusal grants no retry,
/// deadline extension, network authority or refund of an already submitted transport request.
#[derive(Debug, Error)]
pub enum SourceReadError {
    #[error("file source request is no longer active")]
    Revoked,
    #[error("file source read exceeds its bound")]
    Bounds,
    #[error("file source read capacity is in use")]
    Busy,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Default)]
struct SourcePool(Mutex<BTreeSet<Box<[u8]>>>);
impl SourcePool {
    fn reserve(self: &Arc<Self>, group: &[u8]) -> Result<SourceCredit, SourceReadError> {
        if group.is_empty() || group.len() > MAX_GROUP_BYTES {
            return Err(SourceReadError::Bounds);
        }
        let mut held = self.0.lock().unwrap_or_else(|error| error.into_inner());
        // The full group is the key, never a displayed prefix or per-Root counter. A cancelled
        // old Root and a freshly restored owner therefore compete for the same actual slot.
        if held.len() >= PROCESS_READS || held.contains(group) {
            return Err(SourceReadError::Busy);
        }
        held.insert(group.into());
        Ok(SourceCredit {
            pool: self.clone(),
            group: group.into(),
        })
    }
}
struct SourceCredit {
    pool: Arc<SourcePool>,
    group: Box<[u8]>,
}
impl fmt::Debug for SourceCredit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SourceReadCustody { .. }")
    }
}
impl Drop for SourceCredit {
    fn drop(&mut self) {
        self.pool
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(self.group.as_ref());
    }
}

/// One prepared read and its exact prepaid source capacity. Only the owned blocking executor
/// should call `run`; the task grants no membership or permission to sign/deliver its result.
/// There is no Clone, serialization, caller path/key, result or capacity extraction API.
pub struct FileReadTask {
    plan: BlobReadPlan,
    authority: Arc<dyn RequestAuthority>,
    // Declaration order matters: release captured buffers before returning their capacity.
    credit: Arc<SourceCredit>,
}
impl FileReadTask {
    /// Share the SAME reservation with deferred request metadata before starting execution.
    /// The task/completion always keeps its own mandatory owner. This grants no read, signing
    /// or submission authority; it only prevents a panic or detached worker from refunding
    /// capacity while the Root still owns the responder/proof/response (or the reverse).
    pub fn keepalive(&self) -> SharedRequestKeepalive {
        self.credit.clone()
    }

    /// Blocking storage/AEAD/CID work only. Move the entire task into the actual worker closure;
    /// never put its permit in an outer future which can disappear while the closure continues.
    pub fn run(self) -> FileReadCompletion {
        self.run_with(BlobReadPlan::run)
    }
    // The private operation seam pins that refused queued work never enters storage I/O. A
    // corrupt-file test alone could pass even if a completion fence merely hid a real read.
    fn run_with(
        self,
        read: impl FnOnce(BlobReadPlan) -> Result<Option<BlobReadResult>, StorageError>,
    ) -> FileReadCompletion {
        let Self {
            plan,
            authority,
            credit,
        } = self;
        // This synchronous check is the read-admission boundary. Revocation racing AFTER it
        // cannot recall blocking I/O; completion and the actual outgoing effect need new checks.
        let result = if authority.authorized() {
            read(plan).map_err(SourceReadError::from)
        } else {
            drop(plan);
            Err(SourceReadError::Revoked)
        };
        FileReadCompletion {
            result,
            authority,
            _credit: credit,
        }
    }
}
impl fmt::Debug for FileReadTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileReadTask { .. }")
    }
}

/// Exact source custody follows both bytes and storage fences through actor revalidation and
/// page construction. Borrowed access only: owned bytes must never escape their last credit.
pub struct FileReadCompletion {
    result: Result<Option<BlobReadResult>, SourceReadError>,
    authority: Arc<dyn RequestAuthority>,
    _credit: Arc<SourceCredit>,
}
impl FileReadCompletion {
    /// Observe ONLY the weak storage/current-winner fence of a verified positive result. This
    /// deliberately ignores the work authority: its reply-wait lifetime ends on response receive,
    /// before a native driver admits sending. A final Present-response gate must compose THIS
    /// observation with its independent current native/Root/member/deadline authority, retaining
    /// the whole completion as actual response custody. No bytes or read permission are granted.
    /// Missing/error results return false; a signed negative is only a past read observation,
    /// never proof of present-time absence or remote custody. This method supplies no wake seam.
    pub fn storage_is_current(&self) -> bool {
        self.result
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .is_some_and(BlobReadResult::is_current)
    }

    /// A caller must also revalidate its original request/group/session/deadline before effect.
    /// The returned storage fence alone grants no network signature or application mutation.
    /// This is a present-time checked borrow, not a permanent authorization of those bytes.
    /// Recheck at signing/send: a borrowed slice may outlive a later concurrent revocation.
    pub fn result(&self) -> Result<Option<&BlobReadResult>, &SourceReadError> {
        if !self.authority.authorized() {
            return Err(&SourceReadError::Revoked);
        }
        self.result.as_ref().map(Option::as_ref)
    }
}
impl fmt::Debug for FileReadCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileReadCompletion { .. }")
    }
}

/// An executor failure/panic yields None only after its concrete worker's task owner retires.
pub type FileReadFuture = Pin<Box<dyn Future<Output = Option<FileReadCompletion>> + Send>>;

/// Narrow trusted native execution seam, not an IPC or peer capability. Implementations must
/// retain the whole task in actual blocking execution even if this returned future is dropped.
/// A runtime without such an executor must refuse service, never run disk work on the actor.
pub trait FileReadExecutor: Send + Sync + fmt::Debug {
    fn start(&self, task: FileReadTask) -> FileReadFuture;
}

/// An execution adapter around the stable process-owned pool. Constructing/replacing a runtime
/// does not refill the process bound or abandon still-running old workers/results.
#[derive(Clone)]
pub struct SourceReadRuntime {
    pool: Arc<SourcePool>,
    executor: Arc<dyn FileReadExecutor>,
}
impl fmt::Debug for SourceReadRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SourceReadRuntime { .. }")
    }
}
impl SourceReadRuntime {
    #[cfg(test)]
    pub(super) fn testing(executor: Arc<dyn FileReadExecutor>) -> Self {
        Self {
            pool: Arc::new(SourcePool::default()),
            executor,
        }
    }

    pub fn new(executor: Arc<dyn FileReadExecutor>) -> Self {
        static PROCESS_POOL: OnceLock<Arc<SourcePool>> = OnceLock::new();
        Self {
            pool: PROCESS_POOL.get_or_init(Default::default).clone(),
            executor,
        }
    }
    /// Reserve the complete source charge BEFORE preparation can snapshot in-memory content.
    /// This is native capacity only. Authenticate and authorize the exact CID/bound separately
    /// before calling; the read plan cannot establish that the requesting member is permitted.
    pub fn prepare(
        &self,
        group: &[u8],
        store: &dyn BlobStore,
        cid: &Cid,
        max_bytes: usize,
        authority: Arc<dyn RequestAuthority>,
    ) -> Result<FileReadTask, SourceReadError> {
        // The observing authority must NOT retain its publisher, Root/store or signing keys.
        // Scope-owner disappearance must revoke it even while a task/result holds this Arc.
        if !authority.authorized() {
            return Err(SourceReadError::Revoked);
        }
        if max_bytes > MAX_SOURCE_CHUNK
            || BlobReadPlan::memory_bound(max_bytes)? > SOURCE_READ_BYTES
        {
            return Err(SourceReadError::Bounds);
        }
        let credit = Arc::new(self.pool.reserve(group)?);
        let plan = store.prepare_read_bounded(cid, max_bytes)?;
        Ok(FileReadTask {
            plan,
            authority,
            credit,
        })
    }
    /// Transfer the whole affine task to the concrete native blocking executor. A future
    /// cancellation changes observation only; it cannot terminate a detached disk operation.
    pub fn start(&self, task: FileReadTask) -> FileReadFuture {
        self.executor.start(task)
    }
}

#[cfg(test)]
mod tests;
