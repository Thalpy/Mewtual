//! Same-graph cold Current2 owning caller, paired with the task/owner/writer guards,
//! combined invoices and genuine fault tests. This restores only a complete GENESIS head.
//! The seed-absent Windows role additionally requires a new covering scope write; later
//! operational heads need their own reader and can never fall back to this genesis path.
use super::*;
mod settlement;
mod text;
use crate::store::chat_admission::v2_current::{
    ColdV2CurrentPreparation, ColdV2CurrentProgressData,
};
use crate::store::owner_evidence::canonical::current_vacancy::RetiredGroupEntry;
use crate::store::owner_evidence::canonical::v2_registration::cold_current::PreparedColdV2WriterRegistration;
use crate::store::persistence::v2_genesis_activation::{
    cold_writer::PreparedColdV2CurrentWriterReceipt, ColdV2GenesisCurrentHead,
    PreparedColdV2GenesisCapture,
};
use std::cell::RefCell;

/// Retains the actual paid restored graph and every original recovery owner across a
/// finite retry. This handles complete genesis Current2 with its actual seed, or on Windows
/// with independently verified seed absence and a new durable scope barrier. Unknown,
/// partial or later heads refuse instead of being replaced with genesis state.
/// No constructor or successful retry exposes authoring, SDK mutation or a runnable node.
pub struct PreparedColdV2GenesisCurrent<T, R> {
    graph: Graph<T, R>,
    inputs: Option<RuntimeInputs<T, R>>,
    parts: Option<V2GenesisRuntimeParts<T, R>>,
    capture: Option<PreparedColdV2GenesisCapture>,
    head: Option<Arc<ColdV2GenesisCurrentHead>>,
    task: Option<PreparedV2GenesisGroupNamespace>,
    scope: Option<ColdV2CurrentPreparation>,
    writer: Option<PreparedColdV2WriterRegistration>,
    receipt: Option<PreparedColdV2CurrentWriterReceipt>,
    retired_handoff: Option<RetiredGroupEntry>,
    #[cfg(test)]
    after_scope: Option<StoreHook>,
    #[cfg(test)]
    after_receipt_allocation: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    after_pair: Option<StoreHook>,
    #[cfg(test)]
    after_temporary_refund: Option<StoreHook>,
    // LAST after the restored node/inputs, installed generation and their metadata owners.
    fee: Option<CatalogReadReservation>,
}
/// Same actual cold node/owner after final physical and installed-generation checks. This
/// exposes immutable DATA, validation and the distinct original-seed cleanup operation;
/// there is no run, mutable SDK, raw node, authoring or native extractor.
pub struct UnexposedColdV2GenesisCurrent<T, R> {
    node: UnboundV2GenesisNode<T, R>,
    resources: BootstrapGenesisResources,
    head: Arc<ColdV2GenesisCurrentHead>,
    task: PreparedV2GenesisGroupNamespace,
    scope: ColdV2CurrentPreparation,
    writer: PreparedColdV2WriterRegistration,
    receipt: PreparedColdV2CurrentWriterReceipt,
    seed_settlement: Option<
        crate::store::persistence::v2_genesis_activation::settlement::PreparedGenesisSeedSettlement,
    >,
    fee: CatalogReadReservation,
}
opaque_debug!(PreparedColdV2GenesisCurrent, UnexposedColdV2GenesisCurrent);
impl<T, R> Drop for PreparedColdV2GenesisCurrent<T, R> {
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
impl<T, R> Drop for UnexposedColdV2GenesisCurrent<T, R> {
    fn drop(&mut self) {
        self.writer.cancel_original();
        self.scope.cancel_original();
        self.task.cancel_original();
    }
}
impl<T: MeshTransport, R: CryptoRngCore> PreparedColdV2GenesisCurrent<T, R> {
    /// Move the actual paid source/SDK and preexisting runtime inputs. No allocation, read,
    /// scope, execution grant or operational Current follows from this inert constructor.
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
            head: None,
            task: None,
            scope: None,
            writer: None,
            receipt: None,
            retired_handoff: None,
            #[cfg(test)]
            after_scope: None,
            #[cfg(test)]
            after_receipt_allocation: None,
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
            .and_then(|n| n.checked_add(std::mem::size_of::<UnexposedColdV2GenesisCurrent<T, R>>()))
            .and_then(|n| n.checked_add(16 * 1024))
            .ok_or_else(critical_namespaces::refused)?;
        if bytes > WRAPPER_FEE {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    /// One finite explicit attempt. Refusals retain this SAME source, SDK, claim/owner,
    /// registration, cached receipt and node. No network/source execution allowance resets.
    pub fn retry(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
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
            // A failed unrelated save may already have dirtied the global table before
            // even our first Head read. This is DATA persistence only, never authority.
            store.resync_v2_current_scope_inventory(&buffers, rng)?;
            self.advance_storage(store)?;
            self.pair_runtime()?;
            #[cfg(test)]
            if let Some(hook) = self.after_pair.take() {
                hook(store);
            }
            self.resync_bound(store, &buffers, rng)
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
    fn advance_storage(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let source = self.graph.source()?;
        if self.head.is_none() {
            if self.capture.is_none() {
                self.capture = Some(PreparedColdV2GenesisCapture::prepare(store, &source)?);
            }
            let capture = self
                .capture
                .as_mut()
                .ok_or_else(critical_namespaces::refused)?;
            let resources = capture.read_resources();
            let head = store.with_cold_v2_genesis_current_read(&source, |admission| {
                let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
                capture.capture_current(&guard)
            })?;
            self.head = Some(head);
        }
        let head = self
            .head
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if self.task.is_none() {
            self.task = Some(PreparedV2GenesisGroupNamespace::from_cold_current(
                store,
                &source,
                head.clone(),
            )?);
        }
        let task = self
            .task
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        store.prepare_cold_v2_current_scope(&source, task, head.clone(), &mut self.scope)?;
        #[cfg(test)]
        if let Some(hook) = self.after_scope.take() {
            hook(store);
        }
        let scope = self
            .scope
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        // OnceLock original association is established before the scarce registry slot
        // retires. Returned debt/refunds stay outside admission/pins/canonical/Root.
        let retired =
            store.with_cold_owned_current_admission(scope, &source, task, |admission| {
                task.handoff_cold_current_owner(admission)
            })?;
        self.retired_handoff = retired;
        if self.writer.is_none() {
            self.writer = Some(PreparedColdV2WriterRegistration::prepare(
                store, &source, task, scope,
            )?);
        }
        if self.receipt.is_none() {
            self.receipt = Some(PreparedColdV2CurrentWriterReceipt::new(head.clone()));
        }
        let worker = self
            .receipt
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        #[cfg(test)]
        if let Some(hook) = self.after_receipt_allocation.take() {
            worker.after_allocation_for_test(hook);
        }
        // The private Scope implementation invokes install and verify SEQUENTIALLY. This
        // stack-only adapter lets both closed closures borrow the SAME registration; a
        // mistaken overlap refuses via try_borrow instead of panicking or replacing it.
        let writer = RefCell::new(
            self.writer
                .as_mut()
                .ok_or_else(critical_namespaces::refused)?,
        );
        store.finish_cold_v2_genesis_current(
            scope,
            &source,
            task,
            |admission| {
                let mut writer = writer
                    .try_borrow_mut()
                    .map_err(|_| critical_namespaces::refused())?;
                let mut owner = task.lock_cold_owned_current_owner_namespace(admission)?;
                owner.install_cold_writer(&mut writer)?;
                let guard = owner.lock_archive_root_for_cold_writer(&writer)?;
                worker.validate_current(&guard)
            },
            |admission, receipt| {
                let writer = writer
                    .try_borrow()
                    .map_err(|_| critical_namespaces::refused())?;
                let owner = task.lock_cold_owned_current_owner_namespace(admission)?;
                let guard = owner.lock_archive_root_for_cold_writer(&writer)?;
                receipt.validate_under_owned_guard(&guard)
            },
        )
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
    fn resync_bound(
        &mut self,
        store: &ServerStore,
        buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let source = self.graph.source()?;
        let scope = self
            .scope
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        let task = self
            .task
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        match scope.progress_data() {
            ColdV2CurrentProgressData::Unbound => Err(critical_namespaces::refused()),
            ColdV2CurrentProgressData::WriterValidated
            | ColdV2CurrentProgressData::CurrentBound => store.resync_cold_v2_genesis_current(
                scope,
                &source,
                task,
                buffers,
                rng,
                |admission, receipt| {
                    let guard = task.lock_cold_current_proof(admission)?;
                    receipt.validate_under_proof_guard(&guard)
                },
            ),
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
        let task = self
            .task
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let worker = self
            .receipt
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if !Arc::ptr_eq(
            self.head
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?,
            scope.head(),
        ) {
            return Err(critical_namespaces::refused());
        }
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
        store.validate_cold_v2_genesis_current(
            scope,
            &self.graph.source()?,
            task,
            |admission, receipt| {
                worker.require_same_saved_current(receipt)?;
                let owner = task.lock_cold_owned_current_owner_namespace(admission)?;
                let guard = owner.lock_archive_root_for_cold_writer(writer)?;
                receipt.validate_under_owned_guard(&guard)
            },
        )
    }
    /// Move the same recovered node after a new complete physical/installed-writer proof.
    /// Returns None before pairing. A refusal retains the prepared node and original owners;
    /// success supplies the unexposed DATA wrapper, never permission to publish or send.
    pub fn take_current(
        &mut self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<Option<UnexposedColdV2GenesisCurrent<T, R>>, AppError> {
        if !matches!(self.graph, Graph::Node { .. }) {
            return Ok(None);
        }
        if self.head.is_none() || self.receipt.is_none() {
            return Err(critical_namespaces::refused());
        }
        // Also cover a caller which caught an outside-writer panic before retry's ordinary
        // cleanup edge. All possible refund waits still precede the new full final proof.
        drop(self.retired_handoff.take());
        if let Some(writer) = &mut self.writer {
            writer.drain_unused_outside_writers();
        }
        self.validate_ready(layout.checked_store()?)?;
        // Full actual final proof -> infallible moves only, with no refund/allocation/callback.
        let Graph::Node { node, resources } = std::mem::replace(&mut self.graph, Graph::Moving)
        else {
            unreachable!("validated node")
        };
        Ok(Some(UnexposedColdV2GenesisCurrent {
            node,
            resources,
            head: self.head.take().expect("checked head"),
            task: self.task.take().expect("checked task"),
            scope: self.scope.take().expect("checked scope"),
            writer: self.writer.take().expect("checked writer"),
            receipt: self.receipt.take().expect("checked receipt cache"),
            seed_settlement: None,
            fee: self.fee.take().expect("checked fee"),
        }))
    }
}
impl<T, R> UnexposedColdV2GenesisCurrent<T, R> {
    fn source(&self) -> GenesisSourceDataRef<'_> {
        GenesisSourceDataRef {
            seed: self.node.borrow_genesis_seed_data(),
            resources: &self.resources,
        }
    }
    /// Borrow immutable genesis DATA. This is not a live authoring or projection capability.
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.source().data()
    }
    /// Repeat the same source, scope, installed writer and original named-file checks.
    /// Cancellation, a different mount or replacement control objects invalidate this owner.
    pub fn validate_original(&self, layout: &V2CurrentLayoutStore) -> Result<(), AppError> {
        if let Some(result) = super::settlement::validate_after_cleanup(
            &self.settlement_owner_ref()?,
            self.seed_settlement.as_ref(),
            layout,
        ) {
            return result;
        }
        let store = layout.checked_store()?;
        if !Arc::ptr_eq(&self.head, self.scope.head()) {
            return Err(critical_namespaces::refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.fee.validate(pool, WRAPPER_FEE)?;
        store.validate_cold_v2_genesis_current(
            &self.scope,
            &self.source(),
            &self.task,
            |admission, receipt| {
                self.receipt.require_same_saved_current(receipt)?;
                let owner = self
                    .task
                    .lock_cold_owned_current_owner_namespace(admission)?;
                let guard = owner.lock_archive_root_for_cold_writer(&self.writer)?;
                receipt.validate_under_owned_guard(&guard)
            },
        )
    }
}

#[cfg(test)]
mod tests;
