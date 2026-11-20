//! Opt-in, exact pinned-SDK decoder policy. None remains the original legacy decoder.
//! This owns only negative caps/counters; the actual host separately prepays and retains the
//! decoded graph's lease. It cannot authenticate a source, membership or durable Current.
use crate::{MemoryStorage, MemoryStorageError, StorageMap};
use serde::de::DeserializeOwned;
use std::fmt;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};

mod cursor;
mod decode;
mod shape;
pub(crate) mod text_add;
mod text_current;
pub use text_add::{TextAddCandidateGuard, TextAddOperationLimits};
#[cfg(any(test, feature = "test-utils"))]
pub use text_add::test_support::{TextAddTestEvent, TextAddTestObservation};
pub use text_current::{TextCurrentRestoreGuard, TextCurrentRestoreLimits};

/// Negative limits for ONE audited initial founder/first-Add load. Not payment or authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerCheckpointRestoreLimits {
    pub max_typed_bytes: usize,
    pub max_work: usize,
}
#[derive(Debug, Clone, Copy)]
struct Error;
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unsupported bounded SDK restore")
    }
}
impl std::error::Error for Error {}
impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self
    }
}
impl From<Error> for MemoryStorageError {
    fn from(_: Error) -> Self {
        Self::SerializationError
    }
}

/// Requested allocation sums are monotonic, including buffers subsequently freed on this
/// attempt. This pays realloc overlap without guessing that a growth request was in-place.
struct Account {
    bytes: usize,
    work: usize,
    seen: u16,
    facts: shape::Facts,
    text_facts: Option<text_current::shape::Facts>,
}
impl Account {
    fn new(limits: OwnerCheckpointRestoreLimits) -> Result<Self, Error> {
        if limits.max_typed_bytes == 0
            || limits.max_typed_bytes > isize::MAX as usize
            || limits.max_work == 0
        {
            return Err(Error);
        }
        Ok(Self {
            bytes: limits.max_typed_bytes,
            work: limits.max_work,
            seen: 0,
            facts: shape::Facts::default(),
            text_facts: None,
        })
    }
    fn bytes(&mut self, amount: usize) -> Result<(), Error> {
        self.bytes = self.bytes.checked_sub(amount).ok_or(Error)?;
        Ok(())
    }
    fn work(&mut self, amount: usize) -> Result<(), Error> {
        self.work = self.work.checked_sub(amount).ok_or(Error)?;
        Ok(())
    }
    /// Actual Rust1.94 RawVec growth requests for the audited serde Vec/VecDeque visitors
    /// (size_hint=0, then next_element before push). Never apply this to in-place seeds or an
    /// arbitrary custom container visitor. Inline tuples/arrays use no outer Vec request.
    fn sequence(&mut self, count: usize, element: usize) -> Result<(), Error> {
        self.work(count)?;
        if count == 0 || element == 0 {
            return Ok(());
        }
        let mut capacity = if element == 1 {
            8usize
        } else if element <= 1024 {
            4
        } else {
            1
        };
        let mut total = 0usize;
        loop {
            let request = capacity
                .checked_mul(element)
                .filter(|n| *n <= isize::MAX as usize)
                .ok_or(Error)?;
            total = total.checked_add(request).ok_or(Error)?;
            if capacity >= count {
                break;
            }
            capacity = capacity.checked_mul(2).ok_or(Error)?;
        }
        self.bytes(total)
    }
    fn lookup(&mut self, backing: usize, key_length: usize) -> Result<(), Error> {
        // One allocation byte overpays each possible bucket. Each possible candidate comparison
        // pays the full fixed512-byte key slot plus its control probe, not merely the key hash.
        self.work(
            backing
                .checked_mul(513)
                .and_then(|n| n.checked_add(key_length))
                .ok_or(Error)?,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Row {
    Signer,
    Tree,
    Proposals,
    Context,
    Interim,
    Confirmation,
    Epoch,
    OwnIndex,
    Messages,
    Resumption,
    Config,
    OwnLeaves,
    State,
}
const ROWS: [Row; 13] = [
    Row::Signer,
    Row::Tree,
    Row::Proposals,
    Row::Context,
    Row::Interim,
    Row::Confirmation,
    Row::Epoch,
    Row::OwnIndex,
    Row::Messages,
    Row::Resumption,
    Row::Config,
    Row::OwnLeaves,
    Row::State,
];
impl Row {
    fn label(self) -> &'static [u8] {
        match self {
            Self::Signer => b"SignatureKeyPair",
            Self::Tree => b"Tree",
            Self::Proposals => b"ProposalQueueRefs",
            Self::Context => b"GroupContext",
            Self::Interim => b"InterimTranscriptHash",
            Self::Confirmation => b"ConfirmationTag",
            Self::Epoch => b"EpochSecrets",
            Self::OwnIndex => b"OwnLeafNodeIndex",
            Self::Messages => b"MessageSecrets",
            Self::Resumption => b"ResumptionPsk",
            Self::Config => b"MlsGroupJoinConfig",
            Self::OwnLeaves => b"OwnLeafNodes",
            Self::State => b"GroupState",
        }
    }
    fn from_label(label: &[u8]) -> Result<Self, Error> {
        ROWS.into_iter()
            .find(|row| row.label() == label)
            .ok_or(Error)
    }
    fn empty_list(self) -> bool {
        matches!(self, Self::Proposals | Self::OwnLeaves)
    }
    fn bit(self) -> u16 {
        1 << self as u8
    }
}

/// Fixed canonical keys; no serializer/AST/Vec is needed to prepare the allowed set.
struct Key {
    bytes: [u8; 512],
    length: usize,
}
impl Key {
    fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self
            .length
            .checked_add(bytes.len())
            .filter(|n| *n <= self.bytes.len())
            .ok_or(Error)?;
        self.bytes[self.length..end].copy_from_slice(bytes);
        self.length = end;
        Ok(())
    }
    fn byte(&mut self, value: u8) -> Result<(), Error> {
        let digits = [
            b'0' + value / 100,
            b'0' + (value / 10) % 10,
            b'0' + value % 10,
        ];
        self.push(if value >= 100 {
            &digits
        } else if value >= 10 {
            &digits[1..]
        } else {
            &digits[2..]
        })
    }
    fn new(row: Row, group: &[u8; 16], signer: &[u8; 32]) -> Result<Self, Error> {
        let mut key = Self {
            bytes: [0; 512],
            length: 0,
        };
        key.push(row.label())?;
        if row == Row::Signer {
            key.push(b"{\"value\":[")?;
            for (index, byte) in signer
                .iter()
                .chain(b"RustCryptoSignatureKey")
                .chain([8, 7].iter())
                .enumerate()
            {
                if index != 0 {
                    key.push(b",")?;
                }
                key.byte(*byte)?;
            }
            key.push(b"]}")?;
        } else {
            key.push(b"{\"value\":{\"vec\":[")?;
            for (index, byte) in group.iter().enumerate() {
                if index != 0 {
                    key.push(b",")?;
                }
                key.byte(*byte)?;
            }
            key.push(b"]}}")?;
        }
        key.push(&[0, 1])?;
        Ok(key)
    }
    fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}
const ACTIVE: u8 = 0;
const COMPLETE: u8 = 1;
const CANCELLED: u8 = 2;
/// Shared through provider movement/cloning. It intentionally has NO host fee/callback.
/// The independent lease is retained by both actual device/group after successful loading.
pub(crate) struct Attempt {
    phase: AtomicU8,
    account: Mutex<Account>,
    keys: [Key; 13],
    group: [u8; 16],
    signer: [u8; 32],
}
impl fmt::Debug for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerCheckpointRestoreAttempt")
            .finish_non_exhaustive()
    }
}
/// Must remain owned through the synchronous device/group load. Dropping it early closes the
/// provider permanently; neither Complete nor Cancelled restores the legacy None decoder.
#[must_use]
pub struct OwnerCheckpointRestoreGuard {
    attempt: Arc<Attempt>,
    complete: bool,
}
impl OwnerCheckpointRestoreGuard {
    pub fn complete(mut self) -> Result<(), MemoryStorageError> {
        {
            let mut account = self
                .attempt
                .account
                .lock()
                .map_err(|_| MemoryStorageError::SerializationError)?;
            if account.seen != (1 << ROWS.len()) - 1 {
                return Err(Error.into());
            }
            account.finish_shape()?;
            self.attempt
                .phase
                .compare_exchange(ACTIVE, COMPLETE, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| Error)?;
        }
        self.complete = true;
        Ok(())
    }
}
impl Drop for OwnerCheckpointRestoreGuard {
    fn drop(&mut self) {
        if !self.complete {
            let _ = self.attempt.phase.compare_exchange(
                ACTIVE,
                CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}

// Thirteen bounded SDK read-key builders: serde_json starts128 and needs at most256; raw
// key clone<=256; label Vec<=32, then <=288 and <=576 growth. Basic's StorageId uses32?64.
// Also pay its private Vec?SecretVLBytes32 copy, device credential/public copies32+32,
// and GroupId16. These are named actual source paths, not a JSON-byte heap multiplier.
const SDK_FIXED_REQUESTS: usize = 13 * (128 + 256 + 256 + 32 + 288 + 576)
    + 32 + 64 // Basic StorageId's source Vec and growth.
    + 32 // Basic private Vec -> SecretVLBytes copy.
    + 32 + 32 // Device Basic credential and SignaturePublicKey copies.
    + 16 // Strict GroupId16 constructor.
    + 2 * std::mem::size_of::<usize>(); // New ServerGroup CatalogCache's Arc<()> header.

impl MemoryStorage {
    /// Install the opt-in CLOSED initial SDK policy before even the signer's typed getter.
    /// The caller must already own a real graph resource reservation; these caps are DATA only.
    /// Optional draft features are refused because their additional rows/graphs are not audited.
    pub fn begin_owner_checkpoint_restore(
        &self,
        group: &[u8],
        signer: &[u8],
        limits: OwnerCheckpointRestoreLimits,
    ) -> Result<OwnerCheckpointRestoreGuard, MemoryStorageError> {
        self.begin_closed_restore(group, signer, limits, text_current::Family::Initial)
    }
    /// Preserve legacy diagnostics only on the unrestricted path. Backtrace formatting and
    /// host loggers are opaque allocation/work/callbacks, so they must never run under a
    /// closed typed read's values guard. This returns DATA and performs no diagnostics itself.
    pub(crate) fn legacy_read_diagnostics(&self) -> Result<bool, MemoryStorageError> {
        Ok(self.owner_restore.lock().map_err(|_| Error)?.is_none())
    }

    /// Check BEFORE serializer/key-builder allocation at EVERY SDK read entrypoint.
    pub(crate) fn require_owner_read_label(&self, label: &[u8]) -> Result<(), MemoryStorageError> {
        if let Some(attempt) = self.owner_restore.lock().map_err(|_| Error)?.as_ref() {
            if attempt.phase.load(Ordering::Acquire) != ACTIVE {
                return Err(Error.into());
            }
            Row::from_label(label)?;
        }
        Ok(())
    }
    /// Every write wrapper calls this before serialization/key construction; no completed
    /// restore implicitly admits an Add, live ratchet update or another SDK operation.
    pub(crate) fn require_unrestricted(&self) -> Result<(), MemoryStorageError> {
        if self.owner_restore.lock().map_err(|_| Error)?.is_some() {
            Err(Error.into())
        } else {
            Ok(())
        }
    }
    /// Caller holds the SAME actual values guard for policy, shape check and typed decode.
    /// All generic/direct/list/optional getters use this before any missing/default return.
    pub(crate) fn decode_row<V: DeserializeOwned>(
        &self,
        values: &StorageMap,
        label: &[u8],
        key: &[u8],
    ) -> Result<Option<V>, MemoryStorageError> {
        let attempt = self.owner_restore.lock().map_err(|_| Error)?.clone();
        let Some(attempt) = attempt else {
            return values
                .get(key)
                .map(|bytes| serde_json::from_slice(bytes).map_err(Into::into))
                .transpose();
        };
        if attempt.phase.load(Ordering::Acquire) != ACTIVE {
            return Err(Error.into());
        }
        let row = Row::from_label(label)?;
        let expected = &attempt.keys[row as usize];
        let mut account = attempt.account.lock().map_err(|_| Error)?;
        account.work(expected.length.checked_add(key.len()).ok_or(Error)?)?;
        if key != expected.bytes() || account.seen & row.bit() != 0 {
            return Err(Error.into());
        }
        account.lookup(values.allocation_size(), key.len())?;
        let bytes = match values.get(key) {
            Some(bytes) => bytes.as_slice(),
            None if row.empty_list() => b"[]",
            None => return Ok(None),
        };
        account.check_shape(row, bytes, &attempt.group, &attempt.signer, true)?;
        let decoded = decode::from_slice(bytes, &mut account)?;
        account.seen |= row.bit();
        if attempt.phase.load(Ordering::Acquire) != ACTIVE {
            return Err(Error.into());
        }
        Ok(Some(decoded))
    }
}

// Move a fixed key out of each prebuilt Result without a heap or unstable array try_map.
trait TakeKey {
    fn take_key(&mut self) -> Result<Key, Error>;
}
impl TakeKey for Result<Key, Error> {
    fn take_key(&mut self) -> Result<Key, Error> {
        std::mem::replace(self, Err(Error))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
std::thread_local! {
    static BEFORE_WRITER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(crate) fn before_values_writer() {
    let hook = BEFORE_WRITER.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
