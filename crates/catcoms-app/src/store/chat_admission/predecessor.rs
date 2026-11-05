//! Original pre-initialization negative exclusion. This is not a scope-advance permit.
//!
//! An absent/Retired admission has no current row publisher to observe. An actual journal task
//! therefore registers its OWN weak negative under the admission serializer before network work.
//! Store metadata/debt never owns that sender, and expiry never proves durable source absence.
use super::journal_basis::JournalAdmissionPermit;
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::CriticalCatalogReadPool;

const MAX_PREDECESSORS: usize = 5;
mod join_execution;
pub(in crate::store) use join_execution::OriginalJoinExecutionObservation;
// Fixed logical metadata debit, not a physical allocator/RSS measurement. Unlike the typed
// task fee it survives in the inert unresolved slot after the sender and caller disappear.
const REGISTRY_METADATA: usize = 4096;

/// Closed actual consumer kind. Add the archive founder case only with its concrete consumer;
/// this enum is never decoded, public, or a namespace support/old-issuance bypass switch.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    JoinCheckpoint,
}

struct Entry {
    key: ChatAdmissionKey,
    original: Option<Record>,
    mount: Arc<()>,
    source: [u8; 32],
    revision: u64,
    kind: Kind,
    identity: Arc<()>,
    revoker: revocation::Handle,
    negative: RequestCancellation,
    _metadata_fee: CatalogReadReservation,
}

/// Independently charged fixed metadata only: no source key/candidate, transport, Store,
/// session or strong task owner. Each inert exclusion keeps its original metadata fee after
/// task Drop, for the remainder of this physical mount or an exact supported replacement.
pub(super) struct Registry {
    slots: [Option<Entry>; MAX_PREDECESSORS],
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}
impl Registry {
    /// Old generic issuance cannot interpret a dropped task as permission to replace retained
    /// source/debt. Only a future exact intent-backed consumer can explicitly advance it.
    pub(super) fn excludes_generic(&self, key: &ChatAdmissionKey) -> bool {
        self.slots.iter().flatten().any(|entry| &entry.key == key)
    }

    /// Revoke only the exact basis which an ACTUAL row mutation invalidates. No-op Retired or
    /// AlreadyCurrent retries must not call this; they may be older than a newly registered task.
    pub(super) fn before_row_change(&self, key: &ChatAdmissionKey, original: Option<&Record>) {
        for entry in self.slots.iter().flatten() {
            if &entry.key == key && entry.original.as_ref() == original {
                entry.revoker.revoke();
            }
        }
    }

    fn install(
        &mut self,
        prepared: &mut PreparedOriginalPredecessor,
    ) -> Result<Option<Entry>, AppError> {
        let next = prepared.entry.as_ref().ok_or_else(refused)?;
        let index = if let Some(index) = self
            .slots
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.key == next.key))
        {
            let old = self.slots[index].as_ref().ok_or_else(refused)?;
            // SAME durable source may be activated at a genuinely later saved scheduling
            // revision after negative closure. Never renew a revision, kind, basis or sender.
            if old.kind != next.kind
                || old.kind != Kind::JoinCheckpoint
                || old.source != next.source
                || old.original != next.original
                || !Arc::ptr_eq(&old.mount, &next.mount)
                || next.revision <= old.revision
                || !old.negative.is_cancelled()
                || Arc::ptr_eq(&old.identity, &next.identity)
            {
                return Err(refused());
            }
            index
        } else {
            self.slots
                .iter()
                .position(Option::is_none)
                .ok_or_else(refused)?
        };
        // Every fallible check precedes the move; no key/Arc allocation under the writer.
        let next = prepared.entry.take().ok_or_else(refused)?;
        Ok(self.slots[index].replace(next))
    }

    fn close_join(&self, capture: &ChatAdmissionCapture, source: &[u8; 32], revision: u64) {
        for entry in self.slots.iter().flatten() {
            if entry.kind == Kind::JoinCheckpoint
                && entry.key == capture.key
                && entry.original == capture.original
                && Arc::ptr_eq(&entry.mount, &capture.mount_identity)
                && &entry.source == source
                && entry.revision == revision
            {
                entry.revoker.revoke();
            }
        }
    }
}

/// The sole typed task owns its sender. Non-Clone, nonserialized, no source/key or positive
/// permit getter. Keeping an observer, registry identity or Store cannot keep this task alive.
pub(in crate::store) struct OriginalPredecessorTask {
    owner: Arc<revocation::Owner>,
    observer: RequestCancellation,
}
impl OriginalPredecessorTask {
    pub(in crate::store) fn close(&self) {
        self.owner.revoke();
    }
    pub(in crate::store) fn observer(&self) -> RequestCancellation {
        self.observer.clone()
    }
    pub(in crate::store) fn is_closed(&self) -> bool {
        self.observer.is_cancelled()
    }
    fn into_task(prepared: PreparedOriginalPredecessor) -> Self {
        prepared.task
    }
}
impl Drop for OriginalPredecessorTask {
    fn drop(&mut self) {
        self.owner.revoke();
    }
}

pub(in crate::store) struct PreparedOriginalPredecessor {
    task: OriginalPredecessorTask,
    // Dropped after the sole sender/task on a refused preparation. Successful installation
    // transfers this fee to Runtime; source/worker Drop must not refund retained exclusions.
    entry: Option<Entry>,
}
impl PreparedOriginalPredecessor {
    /// Construct ONLY after the concrete caller prepays bounded key/Arc/wakeup metadata. This
    /// prepares negative lifetime, not actual registration, source validity or a disk event.
    pub(in crate::store) fn join_checkpoint(
        capture: &ChatAdmissionCapture,
        source: [u8; 32],
        revision: u64,
        pool: &CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        Self::prepare(capture, source, revision, Kind::JoinCheckpoint, pool)
    }

    fn prepare(
        capture: &ChatAdmissionCapture,
        source: [u8; 32],
        revision: u64,
        kind: Kind,
        pool: &CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        if revision == 0 {
            return Err(refused());
        }
        let metadata_fee = pool.try_reserve(REGISTRY_METADATA)?;
        let owner = revocation::Owner::new();
        let observer = owner
            .observer()
            .with_authority(Arc::new(OriginalBasisAuthority {
                task: owner.observer(),
                mount: capture.mount.clone(),
                admission: capture.admission.clone(),
            }));
        if observer.is_cancelled() {
            return Err(refused());
        }
        Ok(Self {
            entry: Some(Entry {
                key: capture.key.clone(),
                original: capture.original.clone(),
                mount: capture.mount_identity.clone(),
                source,
                revision,
                kind,
                identity: owner.identity(),
                revoker: owner.handle(),
                negative: observer.clone(),
                _metadata_fee: metadata_fee,
            }),
            task: OriginalPredecessorTask { owner, observer },
        })
    }

    /// Move negative lifetime only. This is not evidence of registration; only the concrete
    /// actual caller may construct its activated wrapper after register succeeded.
    pub(in crate::store) fn into_task(self) -> OriginalPredecessorTask {
        OriginalPredecessorTask::into_task(self)
    }
}

#[derive(Debug)]
struct OriginalBasisAuthority {
    task: RequestCancellation,
    mount: RequestCancellation,
    admission: Option<RequestCancellation>,
}
#[async_trait::async_trait]
impl RequestAuthority for OriginalBasisAuthority {
    fn authorized(&self) -> bool {
        !self.task.is_cancelled()
            && !self.mount.is_cancelled()
            && !self
                .admission
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
    }
    async fn revoked(&self) {
        let mut task = self.task.clone();
        let mut mount = self.mount.clone();
        let mut admission = self.admission.clone();
        if self.authorized() {
            tokio::select! {
                _ = task.cancelled() => {},
                _ = mount.cancelled() => {},
                _ = async {
                    match admission.as_mut() {
                        Some(original) => original.cancelled().await,
                        None => std::future::pending::<()>().await,
                    }
                } => {},
            }
        }
    }
}

impl ServerStore {
    /// Original JOIN registration, not a general namespace support callback. The concrete
    /// consumer validates its exact saved catalog source/file/root, with catalog AFTER admission.
    /// A future archive kind must add its actual source/debt consumer together with that kind.
    /// That callback cannot allocate/drop large data/fees, await or re-enter admission getters.
    /// We retain admission after it returns, so another original catalog mutation cannot race
    /// the final recheck/registration even though the callback has released its catalog guard.
    pub(in crate::store) fn register_original_predecessor(
        &self,
        capture: &ChatAdmissionCapture,
        prepared: &mut PreparedOriginalPredecessor,
        confirm: impl FnOnce(&JournalAdmissionPermit<'_>) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        {
            let permit = JournalAdmissionPermit::for_original(self, capture, &runtime);
            permit.validate_original()?;
            confirm(&permit)?;
            permit.validate_original()?;
        }
        let entry = prepared.entry.as_ref().ok_or_else(refused)?;
        if entry.key != capture.key
            || entry.original != capture.original
            || !Arc::ptr_eq(&entry.mount, &capture.mount_identity)
            || prepared.task.is_closed()
        {
            return Err(refused());
        }
        let retired = runtime.predecessors.install(prepared)?;
        drop(runtime);
        // Replaced weak metadata/key backing drops only outside admission/catalog writers.
        drop(retired);
        Ok(())
    }

    /// Stop only this exact source/revision before a dependent scheduling write. A later task
    /// cannot be reached by stale labels/revokers. This is negative-only even for a now-changed
    /// admission basis; it creates no fresh capture/allowance or replacement source.
    pub(in crate::store) fn close_original_join_predecessor(
        &self,
        capture: &ChatAdmissionCapture,
        source: &[u8; 32],
        revision: u64,
    ) -> Result<(), AppError> {
        let runtime = self.chat_mount.locked()?;
        if !Arc::ptr_eq(&capture.mount_identity, &self.chat_mount.identity) {
            return Err(refused());
        }
        runtime.predecessors.close_join(capture, source, revision);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
