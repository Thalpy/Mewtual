//! The deletion core of leaving a server: empty its blob namespace except what Studio still
//! references, then its scratch directories and `kept/`, and remove the resume marker only when
//! everything went.
//!
//! The desktop bridge drives this in three phases so the vault store lock is held only for the
//! first: [`ServerStore::begin_leave_cleanup`] (the complete Studio reference scan, a cache-only
//! handle, and the exact list of CIDs to delete), then [`LeaveCleanup::delete_batch`] repeatedly
//! without the store lock, then [`LeaveCleanup::finish`]. Claims on the namespace and session
//! checks between batches belong to the bridge. Every failure keeps data: an incomplete reference
//! scan deletes no cache file, a failed delete keeps the marker, and only the bytes actually
//! removed are reported as freed.

use super::*;
use catcoms_storage::cache as storage_cache;
use catcoms_storage::Cid;
use std::collections::BTreeSet;

/// What leaving freed on this device.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LeaveReport {
    /// Bytes actually removed (cache files, scratch and Keep).
    pub freed_bytes: u64,
    /// Cache bytes kept because Studio still references them.
    pub kept_bytes: u64,
    /// Entries that could not be removed; the marker stays and cleanup resumes later.
    pub failed: usize,
}

/// One namespace being emptied. `Send`, so the bridge can move it into blocking workers.
pub struct LeaveCleanup {
    dir: PathBuf,
    handle: Box<dyn BlobStore + Send>,
    entries: Vec<(Cid, u64)>,
    protected: BTreeSet<Cid>,
    next: usize,
    report: LeaveReport,
}

impl std::fmt::Debug for LeaveCleanup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeaveCleanup")
            .field("entries", &self.entries.len())
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    /// Phase 1, under the store's exclusive custody: run the complete Studio reference scan, open
    /// a cache-only handle (the protected primary without Keep, so the `kept/` lease stays free)
    /// and enumerate the cache. If the scan fails or the cache cannot be fully listed, nothing in
    /// the cache will be deleted; scratch and `kept/` are still removed by [`LeaveCleanup::finish`]
    /// because nothing Studio references lives there.
    pub fn begin_leave_cleanup(&mut self, key: &str) -> Result<LeaveCleanup, AppError> {
        let dir = self.cache_namespace_dir(key)?;
        let studio_known = self.creative_pinned_cids().is_ok();
        let mut handle = self.cache_only_blob_store(key)?;
        let scanned = handle.cache_scan(true).is_ok_and(|report| report.complete);
        let protected = handle.protected_cids();
        let mut report = LeaveReport::default();
        let entries = match (
            studio_known && scanned,
            handle.cache_index(),
            protected.as_ref(),
        ) {
            (true, Some(index), Some(_)) => index.sizes().iter().map(|(c, n)| (*c, *n)).collect(),
            _ => {
                report.failed += 1; // keep the whole cache and the marker; resume later
                Vec::new()
            }
        };
        Ok(LeaveCleanup {
            dir,
            handle,
            entries,
            protected: protected.unwrap_or_default(),
            next: 0,
            report,
        })
    }
}

impl LeaveCleanup {
    /// Phase 2, no store lock: delete up to `max` enumerated CIDs through the Studio guard.
    /// Returns whether more remain.
    pub fn delete_batch(&mut self, max: usize) -> bool {
        let end = self.next.saturating_add(max).min(self.entries.len());
        for (cid, bytes) in &self.entries[self.next..end] {
            if self.protected.contains(cid) {
                self.report.kept_bytes = self.report.kept_bytes.saturating_add(*bytes);
                continue;
            }
            match self.handle.delete(cid) {
                Ok(true) => {
                    self.report.freed_bytes = self.report.freed_bytes.saturating_add(*bytes)
                }
                // The guard kept it: a Studio hold arrived after enumeration.
                Ok(false) if self.handle.has(cid) => {
                    self.report.kept_bytes = self.report.kept_bytes.saturating_add(*bytes)
                }
                Ok(false) => {}
                // Unknown protection (the vault locked meanwhile) or a third-party handle.
                Err(_) => self.report.failed += 1,
            }
        }
        self.next = end;
        self.next < self.entries.len()
    }

    /// The bridge stopped early (the session changed): count what is left as not removed.
    pub fn abandon(&mut self) {
        self.report.failed += self.entries.len() - self.next;
        self.next = self.entries.len();
    }

    /// Phase 3: drop the handle, remove `staging/`, `incoming/` and `kept/`, and remove the marker
    /// (and the namespace, if empty) only when nothing failed.
    pub fn finish(mut self) -> LeaveReport {
        if self.next < self.entries.len() {
            self.abandon();
        }
        let dir = self.dir.clone();
        drop(self.handle);
        let mut removal = storage_cache::remove_scratch(&dir);
        removal.add(storage_cache::remove_kept_tree(&dir));
        self.report.freed_bytes = self.report.freed_bytes.saturating_add(removal.freed_bytes);
        self.report.failed += removal.failed;
        if self.report.failed == 0 && storage_cache::finish_leaving(&dir).is_err() {
            self.report.failed += 1;
        }
        self.report
    }
}
