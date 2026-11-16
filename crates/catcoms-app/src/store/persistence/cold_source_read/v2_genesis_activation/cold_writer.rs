//! Installed cold writer proof, separate from the earlier task-free ColdHead read and
//! consumed by the real clean/dirty Factory guards, scope latch and owning library caller.
//! Nothing here writes a head, rewrites a scope row or restores an operational SDK.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    ColdOwnedV2CurrentGroupGuard, ColdV2CurrentProofGroupGuard,
};

/// One exact independently paid recovered head and original local writer generation. Its
/// opaque constructor requires a real installed-writer+Root guard, never a decoded head or
/// a negative cancellation token alone. This receipt contains no source graph or sender.
pub(in crate::store) struct ValidatedColdV2CurrentWriter {
    head: Arc<ColdV2GenesisCurrentHead>,
    task: OriginalV2GenesisGroupObservation,
    owner_identity: Arc<()>,
    writer: V2WriterObservation,
}
impl std::fmt::Debug for ValidatedColdV2CurrentWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedColdV2CurrentWriter")
            .finish_non_exhaustive()
    }
}

/// Inert owning receipt cache. It borrows no Store and creates no new body on construction;
/// its existing ColdHead Arc pays the named metadata in the shared cold capture invoice.
pub(in crate::store) struct PreparedColdV2CurrentWriterReceipt {
    saved: Option<Arc<ValidatedColdV2CurrentWriter>>,
    #[cfg(test)]
    after_receipt_allocation: Option<Box<dyn FnOnce() + Send>>,
    // LAST: same actual paid head, never a separate uncharged or label-derived payer.
    head: Arc<ColdV2GenesisCurrentHead>,
}
impl std::fmt::Debug for PreparedColdV2CurrentWriterReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedColdV2CurrentWriterReceipt")
            .finish_non_exhaustive()
    }
}

/// Additional simultaneous requested metadata inside the ACTUAL cold1MiB resource loan.
/// The writer's own8KiB generation/map, source, SDK and scope buffers are independently paid.
pub(in crate::store) const COLD_WRITER_RECEIPT_METADATA_BYTES: usize =
    std::mem::size_of::<PreparedColdV2CurrentWriterReceipt>()
        + 2 * std::mem::size_of::<ValidatedColdV2CurrentWriter>()
        + std::mem::size_of::<OriginalV2GenesisGroupObservation>()
        + 8 * 64
        + 1024;

impl PreparedColdV2CurrentWriterReceipt {
    /// Cached ORIGINAL association only. It neither proves installation nor substitutes
    /// for the actual guard's final physical check.
    pub(in crate::store) fn require_same_saved_current(
        &self,
        saved: &Arc<ValidatedColdV2CurrentWriter>,
    ) -> Result<(), AppError> {
        if !self
            .saved
            .as_ref()
            .is_some_and(|ours| Arc::ptr_eq(ours, saved))
            || !Arc::ptr_eq(&self.head, saved.head())
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(in crate::store) fn after_allocation_for_test(&mut self, hook: Box<dyn FnOnce() + Send>) {
        self.after_receipt_allocation = Some(hook);
    }
    #[cfg(test)]
    pub(in crate::store) fn cached_current_for_test(
        &self,
    ) -> Option<Arc<ValidatedColdV2CurrentWriter>> {
        self.saved.clone()
    }
    pub(in crate::store) fn new(head: Arc<ColdV2GenesisCurrentHead>) -> Self {
        Self {
            saved: None,
            #[cfg(test)]
            after_receipt_allocation: None,
            head,
        }
    }

    /// Source/registration ownership and every physical barrier must already be actual.
    /// Allocate/cache before the LAST full proof so a late refusal retries this SAME result.
    pub(in crate::store) fn validate_current(
        &mut self,
        guard: &ColdOwnedV2CurrentGroupGuard<'_, '_>,
    ) -> Result<Arc<ValidatedColdV2CurrentWriter>, AppError> {
        let admission = guard.scope_admission();
        admission.validate_source_only()?;
        if !Arc::ptr_eq(&self.head, admission.head()) {
            return Err(critical_namespaces::refused());
        }
        self.head.validate_under_archive_root(
            admission.store(),
            admission.source(),
            guard.archive_root(),
        )?;
        guard.validate_source_only()?;
        if self.saved.is_none() {
            let saved = Arc::new(ValidatedColdV2CurrentWriter {
                head: self.head.clone(),
                task: admission.task().observe_original(),
                owner_identity: admission.owner_identity().clone(),
                writer: guard.writer_registration().observe_original(),
            });
            self.saved = Some(saved);
            #[cfg(test)]
            if let Some(hook) = self.after_receipt_allocation.take() {
                hook();
            }
        }
        let saved = self
            .saved
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        saved.validate_under_owned_guard(guard)?;
        Ok(saved.clone())
    }
}

impl ValidatedColdV2CurrentWriter {
    pub(in crate::store) fn head(&self) -> &Arc<ColdV2GenesisCurrentHead> {
        &self.head
    }
    pub(in crate::store) fn writer_observation(&self) -> &V2WriterObservation {
        &self.writer
    }

    /// Cached original associations/negatives only. Scope invokes the concrete full guarded
    /// verifier separately; this method cannot substitute a token for physical/installed proof.
    pub(in crate::store) fn require_original_source_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        head: &Arc<ColdV2GenesisCurrentHead>,
        owner_identity: &Arc<()>,
    ) -> Result<(), AppError> {
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        self.head.retains_original(store, source)?;
        if !Arc::ptr_eq(&self.head, head)
            || !Arc::ptr_eq(&self.owner_identity, owner_identity)
            || self.writer.is_cancelled()
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    /// CLEAN concrete actual installed-writer guard. The registration is borrowed through
    /// Root: no replacement, sender drain or registry mutation may slip through a data ID.
    pub(in crate::store) fn validate_under_owned_guard(
        &self,
        guard: &ColdOwnedV2CurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let admission = guard.scope_admission();
        self.require_original_source_negative(
            admission.store(),
            admission.source(),
            admission.task(),
            admission.head(),
            admission.owner_identity(),
        )?;
        guard.validate_source_only()?;
        guard
            .writer_registration()
            .require_same_observation(&self.writer)?;
        self.head.validate_under_archive_root(
            admission.store(),
            admission.source(),
            guard.archive_root(),
        )?;
        // No fee, allocator, encoder, source-control read or callback after the final proof.
        guard.validate_source_only()?;
        guard
            .writer_registration()
            .require_same_observation(&self.writer)
    }

    /// Existing-writer proof on the SAME possibly dirty latest table. This role can neither
    /// install nor bind. Repeat the exact registry generation after physical source waits.
    pub(in crate::store) fn validate_under_proof_guard(
        &self,
        guard: &ColdV2CurrentProofGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let admission = guard.scope_admission();
        self.require_original_source_negative(
            admission.store(),
            admission.source(),
            admission.task(),
            admission.head(),
            admission.owner_identity(),
        )?;
        guard.validate_source_only()?;
        guard.require_installed_cold_observation(&self.writer)?;
        self.head.validate_under_archive_root(
            admission.store(),
            admission.source(),
            guard.archive_root(),
        )?;
        guard.validate_source_only()?;
        guard.require_installed_cold_observation(&self.writer)
    }
}
