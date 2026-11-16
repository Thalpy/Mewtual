//! Actual partial-genesis Init1 recovery owner. No warm Saved receipt or Current2 cold
//! capture is reconstructed. The same graph/read/T/R/clock survives every refused attempt.
use super::*;
mod pre_initializing;
mod settlement;
use crate::store::chat_admission::v2_current::ColdV2InitializingPreparation;
use crate::store::owner_evidence::canonical::current_vacancy::RetiredGroupEntry;
use crate::store::owner_evidence::canonical::v2_registration::cold_initializing::PreparedColdV2InitializingWriterRegistration;
use crate::store::persistence::v2_genesis_activation::initializing_repair::{
    ColdV2GenesisRepairSeed, PreparedColdV2GenesisRepair, PreparedColdV2GenesisRepairCapture,
};
pub use pre_initializing::PreparedColdV2GenesisPreInitializing;

pub struct PreparedColdV2GenesisInitializing<T, R> {
    graph: Graph<T, R>,
    inputs: Option<RuntimeInputs<T, R>>,
    parts: Option<V2GenesisRuntimeParts<T, R>>,
    capture: Option<PreparedColdV2GenesisRepairCapture>,
    repair_seed: Option<Arc<ColdV2GenesisRepairSeed>>,
    task: Option<PreparedV2GenesisGroupNamespace>,
    scope: Option<ColdV2InitializingPreparation>,
    writer: Option<PreparedColdV2InitializingWriterRegistration>,
    repair: Option<PreparedColdV2GenesisRepair>,
    retired_handoff: Option<RetiredGroupEntry>,
    #[cfg(test)]
    after_head: Option<StoreHook>,
    #[cfg(test)]
    after_pair: Option<StoreHook>,
    #[cfg(test)]
    after_temporary_refund: Option<StoreHook>,
    // LAST: native/file/weak metadata and actual graph owners drain before wrapper fee.
    fee: Option<CatalogReadReservation>,
}
/// Saved repaired genesis with exact same original resources; still no runnable/SDK/raw
/// node/authoring extractor. Initialization retains the original activation seed. Its
/// separate same-owner settlement adapter may remove only that seed after a fresh
/// covering Current2 barrier; a new mount uses its independent absent-seed reader.
pub struct UnexposedRepairedV2GenesisCurrent<T, R> {
    node: UnboundV2GenesisNode<T, R>,
    resources: BootstrapGenesisResources,
    repair_seed: Arc<ColdV2GenesisRepairSeed>,
    task: PreparedV2GenesisGroupNamespace,
    scope: ColdV2InitializingPreparation,
    writer: PreparedColdV2InitializingWriterRegistration,
    repair: PreparedColdV2GenesisRepair,
    // Cleanup history is independent of the original repaired Head. The old physical
    // validator stays strict and is never invoked after this worker admits unlink.
    seed_settlement: Option<
        crate::store::persistence::v2_genesis_activation::settlement::PreparedGenesisSeedSettlement,
    >,
    fee: CatalogReadReservation,
}
opaque_debug!(
    PreparedColdV2GenesisInitializing,
    UnexposedRepairedV2GenesisCurrent
);
impl<T, R> Drop for PreparedColdV2GenesisInitializing<T, R> {
    fn drop(&mut self) {
        if let Some(writer) = &self.writer {
            writer.cancel_original();
        }
        if let Some(scope) = &self.scope {
            scope.cancel_original();
        }
        if let Some(task) = &self.task {
            task.cancel_original();
        }
    }
}
impl<T, R> Drop for UnexposedRepairedV2GenesisCurrent<T, R> {
    fn drop(&mut self) {
        self.writer.cancel_original();
        self.scope.cancel_original();
        self.task.cancel_original();
    }
}
impl<T: MeshTransport, R: CryptoRngCore> PreparedColdV2GenesisInitializing<T, R> {
    /// Inert owning transfer only. The real actual SourceSaved/SDK restore has already
    /// occurred, and no constructor allocation, new Fresh life or historical RNG replay occurs.
    pub fn new(
        seed: RetainedBootstrapGenesisSeed,
        transport: Arc<T>,
        rng: R,
        clock: Arc<dyn Clock + Send>,
    ) -> Self {
        Self {
            graph: Graph::Seed(seed),
            inputs: Some(RuntimeInputs {
                transport,
                rng,
                clock,
            }),
            parts: None,
            capture: None,
            repair_seed: None,
            task: None,
            scope: None,
            writer: None,
            repair: None,
            retired_handoff: None,
            #[cfg(test)]
            after_head: None,
            #[cfg(test)]
            after_pair: None,
            #[cfg(test)]
            after_temporary_refund: None,
            fee: None,
        }
    }
    fn require_invoice() -> Result<(), AppError> {
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
    /// One bounded explicit attempt; errors retain the same file selections, claim, owner,
    /// Head cache, graph and runtime inputs. There is no source/KP/SDK/nonce/grant renewal.
    pub fn retry(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        // Refuse this unsupported native inventory profile before any new wrapper/scope
        // reservation, original-control read or global DATA maintenance. Historical source
        // restoration remains a separate portable API, not a grant to run this consumer.
        crate::store::persistence::v2_genesis_activation::initializing_repair::require_supported_profile()?;
        let store = layout.checked_store()?;
        self.graph.source()?.validate_original(store)?;
        Self::require_invoice()?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        if self.fee.is_none() {
            self.fee = Some(pool.try_reserve(WRAPPER_FEE)?);
        }
        self.fee
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .validate(pool, WRAPPER_FEE)?;
        let buffers = store.reserve_v2_genesis_scope_buffers()?;
        let result = (|| {
            // Global DATA maintenance precedes every first clean role. It never supplies
            // the NEW even-clean scope1 barrier required by the repair callback itself.
            store.resync_v2_current_scope_inventory(&buffers, rng)?;
            if !matches!(self.graph, Graph::Node { .. }) {
                self.advance_storage(store, &buffers, rng)?;
                self.pair_runtime()?;
            }
            #[cfg(test)]
            if let Some(hook) = self.after_pair.take() {
                hook(store);
            }
            store.resync_cold_repaired_v2_genesis_current(
                self.scope
                    .as_mut()
                    .ok_or_else(critical_namespaces::refused)?,
                &self.graph.source()?,
                self.task
                    .as_ref()
                    .ok_or_else(critical_namespaces::refused)?,
                &buffers,
                rng,
            )
        })();
        drop(buffers);
        drop(self.retired_handoff.take());
        if let Some(writer) = &mut self.writer {
            writer.drain_unused_outside_writers();
        }
        result?;
        #[cfg(test)]
        if let Some(hook) = self.after_temporary_refund.take() {
            hook(store);
        }
        self.validate_ready(store)
    }
    fn advance_storage(
        &mut self,
        store: &ServerStore,
        buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let source = self.graph.source()?;
        if self.repair_seed.is_none() {
            if self.capture.is_none() {
                self.capture = Some(PreparedColdV2GenesisRepairCapture::prepare(store, &source)?);
            }
            let capture = self
                .capture
                .as_mut()
                .ok_or_else(critical_namespaces::refused)?;
            let resources = capture.read_resources();
            let seed = store.with_cold_v2_genesis_initializing_read(&source, |admission| {
                let guard = store.lock_cold_initializing_genesis_group(admission, &resources)?;
                capture.capture_initializing(&guard)
            })?;
            self.repair_seed = Some(seed);
        }
        let seed = self
            .repair_seed
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if self.task.is_none() {
            self.task = Some(PreparedV2GenesisGroupNamespace::from_cold_initializing(
                store,
                &source,
                seed.clone(),
            )?);
        }
        let task = self
            .task
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        store.prepare_cold_v2_initializing_scope(&source, task, seed.clone(), &mut self.scope)?;
        let scope = self
            .scope
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        if scope.current_binding().is_ok() {
            return Ok(());
        }
        // Only first setup needs a clean row1 handoff; a retained Head at dirty row2 never
        // enters this lane. Failed writer payment leaves the same exact handoff retryable.
        if self.writer.is_none() {
            self.retired_handoff =
                store.with_cold_initializing_admission(scope, &source, task, |a| {
                    task.handoff_cold_initializing_owner(a)
                })?;
            self.writer = Some(PreparedColdV2InitializingWriterRegistration::prepare(
                store, &source, task, scope,
            )?);
        }
        if self.repair.is_none() {
            self.repair = Some(PreparedColdV2GenesisRepair::new(seed.clone()));
        }
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        let worker = self
            .repair
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        #[cfg(test)]
        let after_head = self.after_head.take();
        store.finish_cold_v2_initializing_current(scope, &source, task, buffers, rng, |admission| {
            let head = {
                let mut owner = task.lock_cold_initializing_owner_namespace(admission)?;
                owner.install_initializing_writer(writer)?;
                let guard = owner.lock_archive_root_for_initializing_writer(writer)?;
                worker.repair(&guard)?
            };
            // Only a deterministic test fault can be armed here; physical subordinate
            // guards have drained, and the real Head already lives in Root's worker cache.
            #[cfg(test)]
            if let Some(hook) = after_head {
                hook(store);
            }
            Ok(head)
        })
    }
    fn pair_runtime(&mut self) -> Result<(), AppError> {
        if matches!(self.graph, Graph::Node { .. }) {
            return Ok(());
        }
        let binding = self
            .scope
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .current_binding()?;
        let Graph::Seed(seed) = &mut self.graph else {
            return Err(critical_namespaces::refused());
        };
        seed.seed.prepare_scope(binding)?;
        if self.parts.is_none() {
            let inputs = self
                .inputs
                .take()
                .ok_or_else(critical_namespaces::refused)?;
            self.parts = Some(V2GenesisRuntimeParts::cold_data(
                inputs.transport,
                inputs.rng,
                inputs.clock,
            ));
        }
        let Graph::Seed(seed) = std::mem::replace(&mut self.graph, Graph::Moving) else {
            unreachable!("checked seed")
        };
        let RetainedBootstrapGenesisSeed { seed, resources } = seed;
        let parts = self.parts.take().expect("paid original parts");
        match seed.prepare_runtime(parts) {
            Ok(prepared) => {
                self.graph = Graph::Node {
                    node: prepared.into_node(),
                    resources,
                };
                Ok(())
            }
            Err(returned) => {
                let (seed, parts, _) = returned.into_originals();
                self.graph = Graph::Seed(RetainedBootstrapGenesisSeed { seed, resources });
                self.parts = Some(parts);
                Err(critical_namespaces::refused())
            }
        }
    }

    fn validate_ready(&self, store: &ServerStore) -> Result<(), AppError> {
        if !matches!(self.graph, Graph::Node { .. })
            || self.inputs.is_some()
            || self.parts.is_some()
        {
            return Err(critical_namespaces::refused());
        }
        let scope = self
            .scope
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let seed = self
            .repair_seed
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if !Arc::ptr_eq(seed, scope.seed()) {
            return Err(critical_namespaces::refused());
        }
        let head = scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        self.repair
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .require_same_saved_head(head)?;
        self.writer
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .require_same_observation(head.writer_observation())?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.fee
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .validate(pool, WRAPPER_FEE)?;
        store.validate_cold_repaired_v2_genesis_current(
            scope,
            &self.graph.source()?,
            self.task
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?,
        )
    }
    /// After a NEW full proof, move only these same owners. No refund/allocation/callback
    /// follows that proof and no partial object escapes on any refusal.
    pub fn take_current(
        &mut self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<Option<UnexposedRepairedV2GenesisCurrent<T, R>>, AppError> {
        if !matches!(self.graph, Graph::Node { .. }) {
            return Ok(None);
        }
        drop(self.retired_handoff.take());
        if let Some(writer) = &mut self.writer {
            writer.drain_unused_outside_writers();
        }
        self.validate_ready(layout.checked_store()?)?;
        let Graph::Node { node, resources } = std::mem::replace(&mut self.graph, Graph::Moving)
        else {
            unreachable!("validated node")
        };
        Ok(Some(UnexposedRepairedV2GenesisCurrent {
            node,
            resources,
            repair_seed: self.repair_seed.take().expect("checked seed"),
            task: self.task.take().expect("checked task"),
            scope: self.scope.take().expect("checked scope"),
            writer: self.writer.take().expect("checked writer"),
            repair: self.repair.take().expect("checked worker"),
            seed_settlement: None,
            fee: self.fee.take().expect("checked fee"),
        }))
    }
}
impl<T, R> UnexposedRepairedV2GenesisCurrent<T, R> {
    fn source(&self) -> GenesisSourceDataRef<'_> {
        GenesisSourceDataRef {
            seed: self.node.borrow_genesis_seed_data(),
            resources: &self.resources,
        }
    }
    /// Immutable original genesis DATA only, not authoring or live projection authority.
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.source().data()
    }
    pub fn validate_original(&self, layout: &V2CurrentLayoutStore) -> Result<(), AppError> {
        if let Some(result) = super::settlement::validate_after_cleanup(
            &self.settlement_owner_ref()?,
            self.seed_settlement.as_ref(),
            layout,
        ) {
            return result;
        }
        let store = layout.checked_store()?;
        if !Arc::ptr_eq(&self.repair_seed, self.scope.seed()) {
            return Err(critical_namespaces::refused());
        }
        let head = self
            .scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        self.repair.require_same_saved_head(head)?;
        self.writer
            .require_same_observation(head.writer_observation())?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.fee.validate(pool, WRAPPER_FEE)?;
        store.validate_cold_repaired_v2_genesis_current(&self.scope, &self.source(), &self.task)
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, not(windows)))]
mod unsupported_tests;
