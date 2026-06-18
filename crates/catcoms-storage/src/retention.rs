//! Adjustable expiry and the garbage-collection engine.
//!
//! Expiry is resolved most-specific-first (per-file → per-server → global, default
//! one month). "Expired" means the cached bytes are dropped and the blob leaves
//! the auto-share set — but its index entry is kept, so it stays re-fetchable by
//! CID. Two review fixes are built in:
//!
//! - **Decorrelated eviction** — each blob gets random jitter added to its
//!   deadline, so a group sharing one default expiry does not all drop a blob in
//!   the same wall-clock window (which would make the "always re-fetchable while
//!   any member holds it" promise false).
//! - **Holder probe** — GC will not evict the *last* copy: it evicts only when a
//!   fresh probe confirms at least `min_holders` other reachable peers hold the
//!   blob. Otherwise it keeps it. A blob with no bytes locally and no known holder
//!   surfaces as [`BlobState::MissingNoHolder`], never a silent gap.

use std::collections::HashMap;

use catcoms_rt::CryptoRngCore;

use crate::blob::BlobStore;
use crate::cid::Cid;
use crate::StorageError;

/// One month in milliseconds (the default expiry horizon).
pub const ONE_MONTH_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// Identifies a server/connection that a blob belongs to (its group id bytes).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServerId(pub Vec<u8>);

/// An expiry setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// Never age-evict (pin forever).
    Never,
    /// Evict this many milliseconds after creation.
    After(u64),
}

/// What kind of blob this is, for retention purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobKind {
    /// Inline media (images/audio) kept locally for embedding — never age-evicted.
    InlineMedia,
    /// A general file — age-evictable once past its expiry.
    File,
}

/// The three-scope expiry policy. Most specific wins.
#[derive(Debug, Clone)]
pub struct ExpiryPolicy {
    global_ms: u64,
    per_server: HashMap<ServerId, u64>,
    per_file: HashMap<Cid, Expiry>,
}

impl Default for ExpiryPolicy {
    fn default() -> Self {
        Self {
            global_ms: ONE_MONTH_MS,
            per_server: HashMap::new(),
            per_file: HashMap::new(),
        }
    }
}

impl ExpiryPolicy {
    /// A policy with a custom global default.
    pub fn with_global(global_ms: u64) -> Self {
        Self {
            global_ms,
            ..Self::default()
        }
    }

    /// Override the expiry for a server.
    pub fn set_server(&mut self, server: ServerId, expiry_ms: u64) {
        self.per_server.insert(server, expiry_ms);
    }

    /// Override the expiry for a single file (by ciphertext CID).
    pub fn set_file(&mut self, cid: Cid, expiry: Expiry) {
        self.per_file.insert(cid, expiry);
    }

    /// The effective expiry for a blob, applying precedence.
    pub fn effective(&self, cid: &Cid, server: &ServerId) -> Expiry {
        if let Some(e) = self.per_file.get(cid) {
            return *e;
        }
        if let Some(ms) = self.per_server.get(server) {
            return Expiry::After(*ms);
        }
        Expiry::After(self.global_ms)
    }
}

/// A metadata row for one stored blob.
#[derive(Clone, Debug)]
pub struct BlobEntry {
    /// The ciphertext content address.
    pub cid: Cid,
    /// Which server it belongs to.
    pub server: ServerId,
    /// Creation time (ms since epoch).
    pub created_at_ms: u64,
    /// Last access time (ms since epoch).
    pub last_access_ms: u64,
    /// Plaintext size in bytes.
    pub size: u64,
    /// Retention class.
    pub kind: BlobKind,
    /// If true, never evicted (archive pin).
    pub pinned: bool,
    /// True once the bytes have been dropped locally (metadata kept; refetchable).
    pub evicted: bool,
    jitter_ms: u64,
}

/// A fresh, network-confirmed count of how many *other* peers hold a blob.
pub trait HolderOracle {
    /// Number of other reachable peers confirmed to hold `cid`.
    fn reachable_holder_count(&self, cid: &Cid) -> usize;
}

/// Where a blob's bytes currently stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobState {
    /// Bytes are present locally.
    Available,
    /// Bytes evicted locally, but at least one peer holds them — re-fetchable.
    EvictedRefetchable,
    /// Bytes evicted locally and no known holder remains — a typed, visible gap.
    MissingNoHolder,
    /// Not tracked by the index.
    Unknown,
}

/// Summary of a GC / cache-clear pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GcReport {
    /// Blobs whose bytes were evicted (metadata kept).
    pub evicted: usize,
    /// Blobs kept because evicting would drop the last copy.
    pub kept_no_replica: usize,
}

/// The local metadata index plus retention logic.
#[derive(Debug)]
pub struct RetentionIndex {
    entries: HashMap<Cid, BlobEntry>,
    policy: ExpiryPolicy,
    max_jitter_ms: u64,
}

impl RetentionIndex {
    /// Build an index over a policy. Jitter is bounded at a quarter of the global
    /// default so eviction decorrelates without unbounded retention.
    pub fn new(policy: ExpiryPolicy) -> Self {
        let max_jitter_ms = policy.global_ms / 4;
        Self {
            entries: HashMap::new(),
            policy,
            max_jitter_ms,
        }
    }

    /// Borrow the policy (e.g. to adjust overrides).
    pub fn policy_mut(&mut self) -> &mut ExpiryPolicy {
        &mut self.policy
    }

    /// Record a newly stored blob, assigning it decorrelation jitter.
    pub fn insert(
        &mut self,
        cid: Cid,
        server: ServerId,
        size: u64,
        kind: BlobKind,
        now_ms: u64,
        rng: &mut impl CryptoRngCore,
    ) {
        let jitter_ms = if self.max_jitter_ms == 0 {
            0
        } else {
            rng.next_u64() % (self.max_jitter_ms + 1)
        };
        self.entries.insert(
            cid,
            BlobEntry {
                cid,
                server,
                created_at_ms: now_ms,
                last_access_ms: now_ms,
                size,
                kind,
                pinned: false,
                evicted: false,
                jitter_ms,
            },
        );
    }

    /// Look up an entry.
    pub fn get(&self, cid: &Cid) -> Option<&BlobEntry> {
        self.entries.get(cid)
    }

    /// Update a blob's last-access time.
    pub fn touch(&mut self, cid: &Cid, now_ms: u64) {
        if let Some(e) = self.entries.get_mut(cid) {
            e.last_access_ms = now_ms;
        }
    }

    /// Pin a blob so it is never evicted.
    pub fn set_pinned(&mut self, cid: &Cid, pinned: bool) {
        if let Some(e) = self.entries.get_mut(cid) {
            e.pinned = pinned;
        }
    }

    /// The wall-clock deadline after which a blob may be age-evicted, or `None`
    /// if it is never age-evicted (inline media, pinned, or `Expiry::Never`).
    pub fn deadline(&self, cid: &Cid) -> Option<u64> {
        let entry = self.entries.get(cid)?;
        if entry.pinned || entry.kind == BlobKind::InlineMedia {
            return None;
        }
        match self.policy.effective(&entry.cid, &entry.server) {
            Expiry::Never => None,
            Expiry::After(base) => Some(entry.created_at_ms + base + entry.jitter_ms),
        }
    }

    /// Whether a blob is past its (jittered) deadline at `now_ms`.
    pub fn is_expired(&self, cid: &Cid, now_ms: u64) -> bool {
        self.deadline(cid).is_some_and(|d| now_ms >= d)
    }

    /// Report a blob's byte/holder state.
    pub fn blob_state(
        &self,
        store: &impl BlobStore,
        oracle: &impl HolderOracle,
        cid: &Cid,
    ) -> BlobState {
        if !self.entries.contains_key(cid) {
            return BlobState::Unknown;
        }
        if store.has(cid) {
            BlobState::Available
        } else if oracle.reachable_holder_count(cid) > 0 {
            BlobState::EvictedRefetchable
        } else {
            BlobState::MissingNoHolder
        }
    }

    /// Garbage-collect blobs past their deadline, dropping bytes but keeping
    /// metadata. Evicts only when at least `min_holders` other peers are
    /// confirmed to hold the blob (never drops the last copy).
    pub fn gc(
        &mut self,
        store: &mut impl BlobStore,
        oracle: &impl HolderOracle,
        now_ms: u64,
        min_holders: usize,
    ) -> Result<GcReport, StorageError> {
        let due: Vec<Cid> = self
            .entries
            .keys()
            .copied()
            .filter(|cid| self.is_expired(cid, now_ms) && !self.is_evicted(cid))
            .collect();
        self.evict_set(store, oracle, &due, min_holders, false)
    }

    /// The user-facing "clear cache of files older than `age_ms`" action. Honors
    /// the holder probe unless `force` is set.
    pub fn clear_older_than(
        &mut self,
        store: &mut impl BlobStore,
        oracle: &impl HolderOracle,
        age_ms: u64,
        now_ms: u64,
        min_holders: usize,
        force: bool,
    ) -> Result<GcReport, StorageError> {
        let due: Vec<Cid> = self
            .entries
            .values()
            .filter(|e| {
                e.kind == BlobKind::File
                    && !e.pinned
                    && !e.evicted
                    && now_ms.saturating_sub(e.created_at_ms) >= age_ms
            })
            .map(|e| e.cid)
            .collect();
        self.evict_set(store, oracle, &due, min_holders, force)
    }

    /// Restore previously-evicted bytes (e.g. an on-demand re-fetch), verifying
    /// they match the CID.
    pub fn rehydrate(
        &mut self,
        store: &mut impl BlobStore,
        cid: &Cid,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        let stored = store.put(bytes)?;
        if stored != *cid {
            // Roll back the mismatched write.
            let _ = store.delete(&stored);
            return Err(StorageError::CidMismatch);
        }
        if let Some(e) = self.entries.get_mut(cid) {
            e.evicted = false;
        }
        Ok(())
    }

    fn is_evicted(&self, cid: &Cid) -> bool {
        self.entries.get(cid).is_some_and(|e| e.evicted)
    }

    fn evict_set(
        &mut self,
        store: &mut impl BlobStore,
        oracle: &impl HolderOracle,
        cids: &[Cid],
        min_holders: usize,
        force: bool,
    ) -> Result<GcReport, StorageError> {
        let mut report = GcReport::default();
        for cid in cids {
            if force || oracle.reachable_holder_count(cid) >= min_holders {
                store.delete(cid)?;
                if let Some(e) = self.entries.get_mut(cid) {
                    e.evicted = true;
                }
                report.evicted += 1;
            } else {
                report.kept_no_replica += 1;
            }
        }
        Ok(report)
    }
}
