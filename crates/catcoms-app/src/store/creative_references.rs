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
pub(super) struct Protection {
    pub(super) generation: Arc<()>,
    pins: Option<CreativeReferences>,
}
pub(super) type SharedProtection = Arc<Mutex<Protection>>;
impl Protection {
    pub(super) fn new(dir: &Path) -> SharedProtection {
        let empty = super::epoch_recovery::inventory::epoch_files_absent(dir).unwrap_or(false);
        Arc::new(Mutex::new(Self {
            generation: Arc::new(()),
            pins: empty.then(CreativeReferences::default),
        }))
    }
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
        let guard = self
            .protection
            .lock()
            .map_err(|_| StorageError::Io("creative reference protection unavailable".into()))?;
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
