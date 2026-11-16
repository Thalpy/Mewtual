//! Complete activation.seed before the first scope row. This owning adapter moves the
//! original task/owner into the existing repaired-current initializer; it never replays
//! Found, decodes another SDK graph, mints a nonce or fabricates warm/row1 provenance.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdV2PreInitializingPreparation, RetiredBornScopeCustody,
};
use crate::store::critical_namespaces::current_layout::group_task::cold_preinitializing::RetiredPreInitializingTaskCustody;
use crate::store::persistence::v2_genesis_activation::pre_initializing::{
    ColdV2PreInitializingSeed, PreparedBornV2GenesisRepairCapture,
    PreparedColdV2PreInitializingCapture,
};

pub struct PreparedColdV2GenesisPreInitializing<T, R> {
    // The graph and original T/R/Clock enter this SAME inner owner in the inert constructor.
    // Custom Drop revokes a still-owned born slot before this graph starts draining.
    inner: PreparedColdV2GenesisInitializing<T, R>,
    capture: Option<PreparedColdV2PreInitializingCapture>,
    origin: Option<Arc<ColdV2PreInitializingSeed>>,
    born_capture: Option<PreparedBornV2GenesisRepairCapture>,
    born: Option<ColdV2PreInitializingPreparation>,
    retired_scope: Option<RetiredBornScopeCustody>,
    retired_task: Option<RetiredPreInitializingTaskCustody>,
    retired_entry: Option<RetiredGroupEntry>,
    // Actual post-commit lost-return cuts, never a row/receipt/permission constructor.
    #[cfg(all(test, windows))]
    fail_after_born: bool,
    #[cfg(all(test, windows))]
    fail_after_handoff: bool,
    #[cfg(all(test, windows))]
    fail_after_conversion: bool,
    // LAST until checked normalization transfers this SAME payment to inner/output.
    fee: Option<CatalogReadReservation>,
}
opaque_debug!(PreparedColdV2GenesisPreInitializing);
impl<T, R> Drop for PreparedColdV2GenesisPreInitializing<T, R> {
    fn drop(&mut self) {
        if let Some(born) = &self.born {
            born.cancel_original();
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> PreparedColdV2GenesisPreInitializing<T, R> {
    /// Inert owning transfer only. Complete journal capture and all actual effects occur
    /// in retry, after explicit real fees; this constructor does not assume row absence.
    pub fn new(
        seed: RetainedBootstrapGenesisSeed,
        transport: Arc<T>,
        rng: R,
        clock: Arc<dyn Clock + Send>,
    ) -> Self {
        Self {
            inner: PreparedColdV2GenesisInitializing::new(seed, transport, rng, clock),
            capture: None,
            origin: None,
            born_capture: None,
            born: None,
            retired_scope: None,
            retired_task: None,
            retired_entry: None,
            #[cfg(all(test, windows))]
            fail_after_born: false,
            #[cfg(all(test, windows))]
            fail_after_handoff: false,
            #[cfg(all(test, windows))]
            fail_after_conversion: false,
            fee: None,
        }
    }
    fn require_combined_invoice() -> Result<(), AppError> {
        let bytes = V2GenesisRuntimeParts::<T, R>::allocation_bytes()
            .checked_add(std::mem::size_of::<Self>())
            .and_then(|n| {
                n.checked_add(std::mem::size_of::<UnexposedRepairedV2GenesisCurrent<T, R>>())
            })
            .and_then(|n| n.checked_add(16 * 1024))
            .ok_or_else(critical_namespaces::refused)?;
        if bytes > WRAPPER_FEE {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    /// An exact continuation. All source/refusal tails stay in this owner, including a
    /// failed first global scope write, lost Born receipt or committed task successor.
    pub fn retry(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        crate::store::persistence::v2_genesis_activation::initializing_repair::require_supported_profile()?;
        let store = layout.checked_store()?;
        self.inner.graph.source()?.validate_original(store)?;
        if let Some(task) = &self.inner.task {
            task.retains_original(store, &self.inner.graph.source()?)?;
            if let Some(born) = &self.born {
                born.require_original_source_negative(store, &self.inner.graph.source()?, task)?;
            }
        }
        Self::require_combined_invoice()?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        if self.fee.is_none() && self.inner.fee.is_none() {
            self.fee = Some(pool.try_reserve(WRAPPER_FEE)?);
        }
        self.fee
            .as_ref()
            .or(self.inner.fee.as_ref())
            .ok_or_else(critical_namespaces::refused)?
            .validate(pool, WRAPPER_FEE)?;
        if self.inner.scope.is_none() {
            let buffers = store.reserve_v2_genesis_scope_buffers()?;
            let result = (|| {
                store.resync_v2_current_scope_inventory(&buffers, rng)?;
                self.advance_birth(store, &buffers, rng)
            })();
            drop(buffers);
            // A committed conversion is recognized from the actual inner slot even if a
            // reply was lost. Cleanup is idempotent and precedes every final inner proof.
            if self.inner.scope.is_some() {
                self.normalize_and_drain(store)?;
            }
            drop(self.retired_entry.take());
            result?;
        }
        self.normalize_and_drain(store)?;
        if self.inner.fee.is_none() {
            self.inner.fee = self.fee.take();
        }
        self.inner.retry(layout, rng)
    }
    fn advance_birth(
        &mut self,
        store: &ServerStore,
        buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let source = self.inner.graph.source()?;
        if self.origin.is_none() {
            if self.capture.is_none() {
                self.capture = Some(PreparedColdV2PreInitializingCapture::prepare(
                    store, &source,
                )?);
            }
            let capture = self
                .capture
                .as_mut()
                .ok_or_else(critical_namespaces::refused)?;
            let resources = capture.read_resources();
            let origin = store.with_cold_v2_genesis_vacant_read(&source, |a| {
                let guard = store.lock_cold_vacant_genesis_group(a, &resources)?;
                capture.capture_vacant(&guard)
            })?;
            self.origin = Some(origin);
        }
        let origin = self
            .origin
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if self.inner.task.is_none() {
            self.inner.task = Some(PreparedV2GenesisGroupNamespace::from_cold_pre_initializing(
                store,
                &source,
                origin.clone(),
            )?);
        }
        let task = self
            .inner
            .task
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        store.prepare_cold_v2_genesis_birth(&source, task, origin.clone(), &mut self.born)?;
        if self.born_capture.is_none() {
            self.born_capture = Some(PreparedBornV2GenesisRepairCapture::prepare(
                store,
                &source,
                origin.clone(),
            )?);
        }
        let capture = self
            .born_capture
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        let resources = capture.read_resources();
        let born = self
            .born
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        store.save_born_v2_genesis_row1(born, &source, task, buffers, rng, |a| {
            let guard = store.lock_durable_born_v2_genesis_group(a, &resources)?;
            capture.capture_born(&guard)
        })?;
        #[cfg(all(test, windows))]
        if std::mem::take(&mut self.fail_after_born) {
            return Err(critical_namespaces::refused());
        }
        self.retired_entry = store.with_born_v2_genesis_admission(born, &source, task, |a| {
            task.handoff_born_v2_genesis_owner(a)
        })?;
        #[cfg(all(test, windows))]
        if std::mem::take(&mut self.fail_after_handoff) {
            return Err(critical_namespaces::refused());
        }
        self.retired_scope = Some(store.convert_born_to_initializing(
            &mut self.born,
            &mut self.inner.scope,
            &source,
            task,
        )?);
        // All effects/proofs are done. This Arc clone cannot allocate or invoke a callback.
        self.inner.repair_seed = Some(
            self.inner
                .scope
                .as_ref()
                .expect("actual converted scope")
                .seed()
                .clone(),
        );
        #[cfg(all(test, windows))]
        if std::mem::take(&mut self.fail_after_conversion) {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    fn normalize_and_drain(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if self.inner.scope.is_none() {
            return Err(critical_namespaces::refused());
        }
        if self.born.is_some() {
            return Err(critical_namespaces::refused());
        }
        let task = self
            .inner
            .task
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        self.retired_task = task.normalize_born_successor(
            store,
            &self.inner.graph.source()?,
            self.inner
                .scope
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?,
        )?;
        // No host/SDK writer is held. Remove EVERY old strong pre-seed/origin/capture link
        // before the normal repair's final positive proof. These values have no revoker
        // ownership that could close the same moved owner, task Life or later output.
        drop(self.retired_entry.take());
        drop(self.retired_scope.take());
        drop(self.retired_task.take());
        drop(self.born_capture.take());
        drop(self.origin.take());
        drop(self.capture.take());
        Ok(())
    }
    /// Returns the ordinary real repaired Current owner only after normalization has
    /// drained old custody and its actual final proof succeeds. Dropping this prepared A
    /// afterward cannot revoke the moved owner: its born slot is already empty.
    pub fn take_current(
        &mut self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<Option<UnexposedRepairedV2GenesisCurrent<T, R>>, AppError> {
        if self.inner.scope.is_none() {
            return Ok(None);
        }
        self.normalize_and_drain(layout.checked_store()?)?;
        self.inner.take_current(layout)
    }
}

#[cfg(test)]
mod tests;
