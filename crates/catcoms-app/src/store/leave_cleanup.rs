//! The deletion core of leaving a server: empty its blob namespace except what Studio still
//! references, then its scratch directories and `kept/`, and remove the resume marker only when
//! everything went.
//!
//! The desktop bridge drives this in three phases so the vault store lock is held only for the
//! first: [`ServerStore::begin_leave_cleanup`] (marker and lease checks, the complete Studio
//! reference scan, a cache-only handle, and the exact list of CIDs to delete), then
//! [`LeaveCleanup::delete_batch`] repeatedly without the store lock, then [`LeaveCleanup::finish`].
//! Claims on the namespace and session checks between batches belong to the bridge.
//!
//! Two checks make it safe against a server that attached the same group before the bridge's
//! claim existed (a rejoin, or a join still being registered while the group's old server runs):
//! the `leaving` marker must still be present (a rejoining attach removes it), and the
//! namespace's attach lock must be free exclusively. Every primary handle holds that lock shared
//! for its lifetime, including a second attach of the group that could not get the Keep lease, so
//! a free lock proves nothing has the namespace open. The exclusive hold, and then the Keep lease,
//! are kept until the end, so nothing can attach meanwhile. Every failure keeps data: an
//! incomplete reference scan deletes no cache file, a failed delete keeps the marker, and only the
//! bytes actually removed count as freed.

use super::*;
use catcoms_storage::cache::{
    self as storage_cache, AttachAttempt, AttachExclusive, KeptLease, LeaseAttempt,
};
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
    /// Whether cleanup ran at all. False when the namespace was in use, never attached, shared
    /// by another server entry, or could not be marked: its files then stay on this device.
    pub cleaned: bool,
}

/// How [`ServerStore::begin_leave_cleanup`] went.
#[derive(Debug)]
pub enum LeaveBegin {
    /// Ready to delete.
    Ready(LeaveCleanup),
    /// No `leaving` marker: a rejoin already took the namespace back, or it was never marked.
    NoMarker,
    /// A primary handle is open (the attach lock is held shared) or a Keep store has not been
    /// dropped yet: the namespace is attached, or the leaving server's own handle is still being
    /// dropped. The two look the same here, so nothing is deleted and the marker stays.
    Busy,
    /// The attach lock, `kept/` or its lease could not be opened. Nothing deleted; the marker
    /// stays.
    Unavailable,
}

/// One namespace being emptied. `Send`, so the bridge can move it into blocking workers.
pub struct LeaveCleanup {
    dir: PathBuf,
    handle: Box<dyn BlobStore + Send>,
    /// The exclusive attach hold that proves no other handle is open; released only in
    /// [`LeaveCleanup::finish`] (or with the value, on an abandoned cleanup).
    attach: Option<AttachExclusive>,
    lease: Option<KeptLease>,
    entries: Vec<(Cid, u64)>,
    protected: BTreeSet<Cid>,
    next: usize,
    abandoned: bool,
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
    /// Phase 1, under the store's exclusive custody. Checks the marker, takes the attach lock
    /// exclusively and then the Keep lease (see the module docs; the same order every attach
    /// uses), runs the complete Studio reference scan, opens a cache-only handle (the protected
    /// primary without Keep or a shared attach hold, so it competes for neither) and enumerates
    /// the cache. If the scan fails or the cache cannot be fully listed, nothing in the cache will
    /// be deleted; scratch and `kept/` are still removed by [`LeaveCleanup::finish`] because
    /// nothing Studio references lives there.
    pub fn begin_leave_cleanup(&mut self, key: &str) -> Result<LeaveBegin, AppError> {
        let dir = self.cache_namespace_dir(key)?;
        if !storage_cache::has_leaving_marker(&dir) {
            return Ok(LeaveBegin::NoMarker);
        }
        let attach = match storage_cache::attach_exclusive(&dir) {
            AttachAttempt::Held(attach) => attach,
            AttachAttempt::Busy => return Ok(LeaveBegin::Busy),
            AttachAttempt::Failed => return Ok(LeaveBegin::Unavailable),
        };
        // A Keep store outlives its primary by a moment while it drops (its lease field drops
        // last), so a held lease after a free attach lock is a handle still going away.
        let lease = match storage_cache::lock_kept(&dir) {
            LeaseAttempt::Held(lease) => lease,
            LeaseAttempt::Busy => return Ok(LeaveBegin::Busy),
            LeaseAttempt::Failed => return Ok(LeaveBegin::Unavailable),
        };
        let studio_known = self.creative_pinned_cids().is_ok();
        let mut handle = self.cache_only_blob_store(key, &attach)?;
        let scanned = handle.cache_scan(true).is_ok_and(|report| report.complete);
        let protected = handle.protected_cids();
        let mut report = LeaveReport {
            cleaned: true,
            ..LeaveReport::default()
        };
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
        Ok(LeaveBegin::Ready(LeaveCleanup {
            dir,
            handle,
            attach: Some(attach),
            lease,
            entries,
            protected: protected.unwrap_or_default(),
            next: 0,
            abandoned: false,
            report,
        }))
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

    /// The bridge stopped early (the session changed): count what is left as not removed. Phase 3
    /// is then skipped too, so a locked or replaced session never has `kept/` emptied under it.
    pub fn abandon(&mut self) {
        self.report.failed += (self.entries.len() - self.next).max(1);
        self.next = self.entries.len();
        self.abandoned = true;
    }

    /// Phase 3: drop the handle, remove `staging/`, `incoming/` and `kept/` (under the lease taken
    /// in phase 1), and only when nothing failed remove the attach lock file, then the marker and
    /// the namespace (if empty). After a failure the attach hold simply drops with `self`; its
    /// file and the marker stay for the next unlock.
    pub fn finish(mut self) -> LeaveReport {
        if self.next < self.entries.len() {
            self.abandon();
        }
        let dir = self.dir.clone();
        drop(self.handle);
        if self.abandoned {
            return self.report; // the holds drop here; the marker stays for the next unlock
        }
        let mut removal = storage_cache::remove_scratch(&dir);
        removal.add(storage_cache::remove_kept_tree_held(
            &dir,
            self.lease.take(),
        ));
        self.report.freed_bytes = self.report.freed_bytes.saturating_add(removal.freed_bytes);
        self.report.failed += removal.failed;
        if self.report.failed > 0 {
            return self.report;
        }
        // The lock file has to go before the directory can. The bridge's claim still keeps every
        // in-process attach out between releasing the hold and removing the namespace.
        let unlocked = self
            .attach
            .take()
            .map_or(Ok(()), storage_cache::remove_attach_lock);
        if unlocked.is_err() || storage_cache::finish_leaving(&dir).is_err() {
            self.report.failed += 1;
        }
        self.report
    }
}
