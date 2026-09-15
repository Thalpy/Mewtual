//! Derived, mount-local protection for held bytes. This is NOT a new durable pin journal or
//! circulation-expiry policy. Only an exclusive complete scan may subtract holds; write paths
//! add them before I/O. Unknown/corrupt/oversized state refuses deletion, never means empty.

use super::*;
use catcoms_replication::{studio, DomainOp, LogicalDocument};
use catcoms_storage::{
    kept::{KeepPlan, KeptFiles},
    Cid,
};
use catcoms_wire::DocType;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// Local reference-cache rail, not replicated admission or a new document limit. Exhausting it
/// stops reclamation until a complete bounded scan is possible; writes do not lose durability.
pub const MAX_CREATIVE_REFERENCES: usize = 65_536;

#[derive(Clone, Default)]
pub struct CreativeReferences {
    groups: BTreeMap<Vec<u8>, BTreeSet<Cid>>,
    count: usize,
}
impl std::fmt::Debug for CreativeReferences {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreativeReferences")
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}
impl CreativeReferences {
    /// Full-group namespace, unioned across numeric server aliases. This is a read-only report,
    /// not permission to delete: it can become stale as soon as the exclusive store is released.
    pub fn for_group(&self, group: &[u8]) -> impl Iterator<Item = &Cid> {
        self.groups.get(group).into_iter().flatten()
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub(super) fn add(
        &mut self,
        group: &[u8],
        cids: impl IntoIterator<Item = [u8; 32]>,
    ) -> Result<(), AppError> {
        if group.is_empty() || group.len() > 256 {
            return Err(invalid());
        }
        for cid in cids {
            let cid = Cid::from_bytes(cid);
            if self.groups.get(group).is_some_and(|set| set.contains(&cid)) {
                continue;
            }
            if self.count == MAX_CREATIVE_REFERENCES {
                return Err(invalid());
            }
            self.groups.entry(group.to_vec()).or_default().insert(cid);
            self.count += 1;
        }
        Ok(())
    }
}
/// Job-owned holds for references that are not durable yet. A complete scan derives its set from
/// durable state alone, so a reference created by an in-flight detached job would otherwise be
/// deleted the moment that scan installs. These entries are NOT cleared by `unknown`, NOT
/// subtractable by an install, and are consulted by every deletion.
struct TransientHold {
    owner: std::sync::Weak<()>,
    group: Vec<u8>,
    cids: BTreeSet<Cid>,
}

// Consumed by the overlay commit path (design I-3), which lands with the runtime on this
// branch. Remove this marker with that commit; it exposes no callable surface today.
#[allow(dead_code)]
/// At most this many live job-owned holds. Exhaustion refuses new admission; it never disables
/// unrelated reclamation by marking the whole store unknown.
pub(super) const MAX_TRANSIENT_HOLD_OWNERS: usize = 8;

pub(super) struct Protection {
    pub(super) generation: Arc<()>,
    pins: Option<CreativeReferences>,
    transient: Vec<TransientHold>,
}
pub(super) type SharedProtection = Arc<Mutex<Protection>>;
impl Protection {
    pub(super) fn new(dir: &Path) -> SharedProtection {
        let empty = super::epoch_recovery::inventory::epoch_files_absent(dir).unwrap_or(false);
        Arc::new(Mutex::new(Self {
            generation: Arc::new(()),
            pins: empty.then(CreativeReferences::default),
            transient: Vec::new(),
        }))
    }
    /// Unknown durable protection. Job-owned holds survive: their references are not derivable
    /// from durable state, so dropping them here would defeat their entire purpose.
    pub(super) fn unknown(&mut self) {
        self.generation = Arc::new(());
        self.pins = None;
    }
    pub(super) fn install(
        &mut self,
        generation: &Arc<()>,
        pins: CreativeReferences,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(generation, &self.generation) {
            return Err(invalid());
        }
        self.pins = Some(pins);
        Ok(())
    }
    /// Drop entries whose owning job has finished. Called before every hold and every deletion,
    /// so a cancelled worker's hold disappears exactly when the worker itself does.
    fn reap(&mut self) {
        self.transient.retain(|hold| hold.owner.strong_count() != 0);
    }
    fn transient_holds(&self, group: &[u8], cid: &Cid) -> bool {
        self.transient.iter().any(|hold| {
            hold.owner.strong_count() != 0 && hold.group == group && hold.cids.contains(cid)
        })
    }
    #[allow(dead_code)]
    fn live_transient_cids(&self) -> usize {
        self.transient
            .iter()
            .filter(|hold| hold.owner.strong_count() != 0)
            .map(|hold| hold.cids.len())
            .sum()
    }
}

/// Releases its hold when dropped, so a cancelled waiter cannot free protection that the actual
/// worker still needs. Moved through every detached stage and result alongside the permit.
#[allow(dead_code)]
pub(crate) struct CreativeHold {
    owner: Arc<()>,
    protection: SharedProtection,
}
impl std::fmt::Debug for CreativeHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CreativeHold { .. }")
    }
}
impl Drop for CreativeHold {
    fn drop(&mut self) {
        if let Ok(mut state) = self.protection.lock() {
            let owner = &self.owner;
            state.transient.retain(|hold| {
                !hold
                    .owner
                    .upgrade()
                    .is_some_and(|live| Arc::ptr_eq(&live, owner))
            });
        }
    }
}
impl ServerStore {
    pub(super) fn hold_creative(&self, group: &[u8], cids: Result<BTreeSet<[u8; 32]>, AppError>) {
        // Poisoned mutexes remain fail-closed, including in BlobStore::delete.
        if let Ok(mut state) = self.creative_protection.lock() {
            state.generation = Arc::new(());
            match cids {
                Ok(cids) => {
                    if let Some(pins) = state.pins.as_mut() {
                        if pins.add(group, cids).is_err() {
                            state.pins = None;
                        }
                    }
                }
                Err(_) => state.pins = None,
            }
        }
    }
    pub(crate) fn hold_creative_operation(&self, document: &LogicalDocument, operation: &DomainOp) {
        if matches!(
            document.doc_type,
            DocType::StudioIndex | DocType::StudioObject
        ) {
            let refs = if operation.doc_type == document.doc_type
                && operation.logical_key == document.logical_key
            {
                studio::operation_blob_cid(operation)
                    .map(|cid| cid.into_iter().collect())
                    .map_err(|_| invalid())
            } else {
                Err(invalid())
            };
            self.hold_creative(&document.server_id, refs);
        } else if document.doc_type != DocType::DocRegistry {
            self.hold_creative(&document.server_id, Err(invalid()));
        }
    }
    /// Protect references that no durable record names yet, for the lifetime of one job.
    ///
    /// Bounds are checked BEFORE any entry is installed, so exhaustion never leaves a partial
    /// hold, and it refuses the caller rather than marking the store unknown: a caller must not be
    /// able to disable unrelated reclamation. That rule is about new admission only; the
    /// fail-closed unknown path after uncertain durable I/O is unaffected.
    ///
    /// This is not durable protection. The caller must transfer these references to the ordinary
    /// conservative holds before releasing the returned guard; see the overlay commit path.
    #[allow(dead_code)]
    pub(crate) fn hold_creative_transient(
        &self,
        group: &[u8],
        cids: BTreeSet<[u8; 32]>,
    ) -> Result<CreativeHold, AppError> {
        if group.is_empty() || group.len() > 256 {
            return Err(invalid());
        }
        let cids: BTreeSet<Cid> = cids.into_iter().map(Cid::from_bytes).collect();
        let mut state = self.creative_protection.lock().map_err(|_| invalid())?;
        state.reap();
        if state.transient.len() >= MAX_TRANSIENT_HOLD_OWNERS {
            return Err(invalid());
        }
        // Transient and durable references share the existing rail. Duplicates are counted, not
        // hidden: a caller cannot obtain unbounded protection by repeating the same CID.
        let durable = state.pins.as_ref().map_or(0, CreativeReferences::len);
        if durable
            .checked_add(state.live_transient_cids())
            .and_then(|held| held.checked_add(cids.len()))
            .is_none_or(|total| total > MAX_CREATIVE_REFERENCES)
        {
            return Err(invalid());
        }
        let owner = Arc::new(());
        state.transient.push(TransientHold {
            owner: Arc::downgrade(&owner),
            group: group.to_vec(),
            cids,
        });
        Ok(CreativeHold {
            owner,
            protection: self.creative_protection.clone(),
        })
    }

    pub(crate) fn creative_references_known(&self) -> bool {
        self.creative_protection
            .lock()
            .is_ok_and(|state| state.pins.is_some())
    }
    /// Scan authenticated retained source, pending intents, and both retained/staged recovery.
    /// No blobs are fetched/read, no expiry enforced, and no records deleted. Must run off the
    /// async executor under the mounted store's existing exclusive lifecycle custody.
    pub fn creative_pinned_cids(&mut self) -> Result<CreativeReferences, AppError> {
        self.creative_protection
            .lock()
            .map_err(|_| invalid())?
            .unknown();
        let mut scan = self.scan_epoch_storage_with_studio()?;
        scan.collect_creative_references()?;
        while !scan.step()?.complete {}
        scan.finish_creative_references()
    }
}

pub(super) fn recovery_cids(
    document: &LogicalDocument,
    state: &EpochRecoveryState,
) -> Result<BTreeSet<[u8; 32]>, AppError> {
    let mut refs = BTreeSet::new();
    if matches!(
        document.doc_type,
        DocType::StudioIndex | DocType::StudioObject
    ) {
        for snapshot in state.retained().chain(state.staged()) {
            refs.extend(
                studio::StudioRecovery::inspect_vault_references(snapshot, document)
                    .map_err(|_| invalid())?,
            );
        }
    } else if document.doc_type != DocType::DocRegistry {
        return Err(invalid());
    }
    Ok(refs)
}

/// Wrap OUTSIDE kept copies: deleting a held cache entry must consult Studio even if a second
/// handle or a fileshare manifest happens to name the same CID. Explicit forget_kept affects
/// only its separately owned kept copy; it never removes the primary frame blob.
pub(super) struct ProtectedBlobs {
    pub(super) inner: Box<dyn BlobStore + Send>,
    pub(super) group: Vec<u8>,
    pub(super) protection: SharedProtection,
}
impl BlobStore for ProtectedBlobs {
    fn delete(&mut self, cid: &Cid) -> Result<bool, StorageError> {
        let mut guard = self
            .protection
            .lock()
            .map_err(|_| StorageError::Io("creative reference protection unavailable".into()))?;
        guard.reap();
        // A live job-owned hold protects references no durable record names yet. Check it even
        // when durable protection is unknown, and before consulting the installed set.
        if guard.transient_holds(&self.group, cid) {
            return Ok(false);
        }
        let pins = guard.pins.as_ref().ok_or_else(|| {
            StorageError::Io("creative references need a complete scan; blob retained".into())
        })?;
        if pins
            .groups
            .get(&self.group)
            .is_some_and(|set| set.contains(cid))
        {
            return Ok(false);
        }
        // Keep this guard THROUGH unlink: check-then-unlock would race a metadata writer's hold.
        self.inner.delete(cid)
    }
    fn kept_files(&self) -> KeptFiles {
        self.inner.kept_files()
    }
    fn begin_keep(&mut self, p: KeepPlan) -> Result<u64, StorageError> {
        self.inner.begin_keep(p)
    }
    fn put_keep(&mut self, t: u64, b: &[u8]) -> Result<(), StorageError> {
        self.inner.put_keep(t, b)
    }
    fn finish_keep(&mut self, t: u64) -> Result<(), StorageError> {
        self.inner.finish_keep(t)
    }
    fn abort_keep(&mut self, t: u64) -> Result<(), StorageError> {
        self.inner.abort_keep(t)
    }
    fn forget_kept(&mut self, c: &Cid) -> Result<(), StorageError> {
        self.inner.forget_kept(c)
    }
    fn is_persistent(&self) -> bool {
        self.inner.is_persistent()
    }
    fn put(&mut self, b: &[u8]) -> Result<Cid, StorageError> {
        self.inner.put(b)
    }
    fn get(&self, c: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
        self.inner.get(c)
    }
    fn get_bounded(&self, c: &Cid, m: usize) -> Result<Option<Vec<u8>>, StorageError> {
        self.inner.get_bounded(c, m)
    }
    fn has(&self, c: &Cid) -> bool {
        self.inner.has(c)
    }
    fn cids(&self) -> Vec<Cid> {
        self.inner.cids()
    }
    fn put_staged(&mut self, b: &[u8]) -> Result<Cid, StorageError> {
        self.inner.put_staged(b)
    }
    fn promote_staged(&mut self, c: &Cid) -> Result<bool, StorageError> {
        self.inner.promote_staged(c)
    }
    fn promote_staged_bounded(&mut self, c: &Cid, m: usize) -> Result<bool, StorageError> {
        self.inner.promote_staged_bounded(c, m)
    }
    fn drop_staged(&mut self, c: &Cid) -> Result<bool, StorageError> {
        self.inner.drop_staged(c)
    }
    fn clear_staging(&mut self) -> Result<usize, StorageError> {
        self.inner.clear_staging()
    }
}
fn invalid() -> AppError {
    AppError::Invalid("creative reference scan incomplete, unsupported or over bound".into())
}

#[cfg(test)]
mod tests;
