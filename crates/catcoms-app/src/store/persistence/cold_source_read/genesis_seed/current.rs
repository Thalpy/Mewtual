//! Actual seed-present Current2 owning caller, paired with the physical namespace, scope
//! and artifact/writer modules. This is an unexposed library boundary, not native activation.
//!
//! This moves the SAME paid restored graph into an UNBOUND node. It exposes no Server,
//! authoring/SDK operation, governor, projection issuer or native result. The activation seed
//! remains present through initial Current activation. The distinct settlement child can
//! later remove it under its actual covering barrier; later operational heads remain separate.
use super::*;
use crate::store::chat_admission::v2_current::V2CurrentPreparation;
use crate::store::critical_namespaces::archive_bootstrap::founder::plan::{
    CompletedWarmFounderOrigin, WarmFounderCurrentReservation,
};
use crate::store::critical_namespaces::{
    current_layout::{PreparedV2GenesisGroupNamespace, V2CurrentLayoutStore},
    read_pool::CatalogReadReservation,
};
use crate::store::owner_evidence::canonical::v2_registration::PreparedV2WriterRegistration;
use crate::store::persistence::v2_genesis_activation::PreparedV2GenesisActivation;
use catcoms_rt::{Clock, MeshTransport};
use catcoms_sync::owner_v2::{UnboundV2GenesisNode, V2GenesisRuntimeParts};

const WRAPPER_FEE: usize = 64 * 1024;

// One-shot test callbacks are owned outside all writers; this alias changes neither their
// allocation nor their drop order and has no production representation.
#[cfg(test)]
type StoreHook = Box<dyn FnOnce(&ServerStore) + Send>;

/// Existing input allocations belong to the host. This inert owner creates no Box, calls no
/// transport/RNG/clock, and supplies no source/scope/current authority of its own.
struct RuntimeInputs<T, R> {
    transport: Arc<T>,
    rng: R,
    clock: Arc<dyn Clock + Send>,
}

enum Graph<T, R> {
    Seed(RetainedBootstrapGenesisSeed),
    Node {
        node: UnboundV2GenesisNode<T, R>,
        resources: BootstrapGenesisResources,
    },
    // Exclusive field-move state only; no source accessor or storage path accepts it.
    // A panic during an outside-writer callback cannot later revive a different graph.
    Moving,
}
impl<T, R> Graph<T, R> {
    fn source(&self) -> Result<GenesisSourceDataRef<'_>, AppError> {
        match self {
            Self::Seed(seed) => Ok(seed.borrow_source_data()),
            Self::Node { node, resources } => Ok(GenesisSourceDataRef {
                seed: node.borrow_genesis_seed_data(),
                resources,
            }),
            Self::Moving => Err(critical_namespaces::refused()),
        }
    }
}

/// Original ownership across every refusal. All fallible work is in `retry`; constructing
/// this inert value allocates no new backing and cannot make its source current. A separately
/// paid runtime/layout allowance precedes new Boxes/observers and all filesystem effects.
pub struct PreparedV2GenesisCurrent<T, R> {
    graph: Graph<T, R>,
    inputs: Option<RuntimeInputs<T, R>>,
    parts: Option<V2GenesisRuntimeParts<T, R>>,
    warm_origin: Option<CompletedWarmFounderOrigin>,
    warm_reservation: Option<WarmFounderCurrentReservation>,
    activation: Option<PreparedV2GenesisActivation>,
    task: Option<PreparedV2GenesisGroupNamespace>,
    scope: Option<V2CurrentPreparation>,
    // Only the actual retry_directory successful return sets this private phase latch.
    // It never substitutes for the real SavedDirectory proof checked by later guards.
    directory_completed: bool,
    // Writer is cancelled by Drop BEFORE graph/T/R payload fields start draining.
    writer: Option<PreparedV2WriterRegistration>,
    #[cfg(test)]
    after_parts: Option<Box<dyn FnOnce() + Send>>,
    // Deterministically stop the NEXT actual scope write after the real head is saved and
    // its subordinate guards drain. This never supplies a replacement writer or receipt.
    #[cfg(test)]
    after_head: Option<StoreHook>,
    #[cfg(test)]
    after_pair: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    after_temporary_refund: Option<Box<dyn FnOnce() + Send>>,
    // LAST: node/parts/writer/layout and their separately paid resource owners drain first.
    fee: Option<CatalogReadReservation>,
}

/// Actual saved genesis Current2 plus its SAME graph/runtime/resources. Still unexposed
/// and unbound: there is intentionally no run, mutable SDK, raw node or Server extractor.
pub struct UnexposedV2GenesisCurrent<T, R> {
    node: UnboundV2GenesisNode<T, R>,
    resources: BootstrapGenesisResources,
    activation: PreparedV2GenesisActivation,
    task: PreparedV2GenesisGroupNamespace,
    scope: V2CurrentPreparation,
    writer: PreparedV2WriterRegistration,
    // Independent actual cleanup history; no node/source owner moves on a refusal.
    seed_settlement: Option<
        crate::store::persistence::v2_genesis_activation::settlement::PreparedGenesisSeedSettlement,
    >,
    // LAST; actual source/SDK/activation/task/writer payments remain independently retained.
    fee: CatalogReadReservation,
}

impl<T, R> Drop for PreparedV2GenesisCurrent<T, R> {
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
impl<T, R> Drop for UnexposedV2GenesisCurrent<T, R> {
    fn drop(&mut self) {
        self.writer.cancel_original();
        self.scope.cancel_original();
        self.task.cancel_original();
    }
}
macro_rules! opaque_debug {
    ($($ty:ident),* $(,)?) => {$(
        impl<T,R> std::fmt::Debug for $ty<T,R> {
            fn fmt(&self, f:&mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )*};
}
opaque_debug!(PreparedV2GenesisCurrent, UnexposedV2GenesisCurrent);

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2GenesisCurrent<T, R> {
    pub fn new(
        seed: RetainedBootstrapGenesisSeed,
        transport: Arc<T>,
        rng: R,
        clock: Arc<dyn Clock + Send>,
    ) -> Self {
        Self::with_runtime(
            seed,
            Some(RuntimeInputs {
                transport,
                rng,
                clock,
            }),
            None,
        )
    }

    /// Only the closed fresh-founder driver supplies original warm runtime
    /// custody. Checked scope pairing still occurs AFTER actual Current storage.
    /// This constructor allocates nothing and confers no positive authority.
    pub(in crate::store) fn from_original_runtime(
        seed: RetainedBootstrapGenesisSeed,
        parts: V2GenesisRuntimeParts<T, R>,
        origin: CompletedWarmFounderOrigin,
    ) -> Self {
        let mut prepared = Self::with_runtime(seed, None, Some(parts));
        prepared.warm_origin = Some(origin);
        prepared
    }

    fn with_runtime(
        seed: RetainedBootstrapGenesisSeed,
        inputs: Option<RuntimeInputs<T, R>>,
        parts: Option<V2GenesisRuntimeParts<T, R>>,
    ) -> Self {
        Self {
            graph: Graph::Seed(seed),
            inputs,
            parts,
            warm_origin: None,
            warm_reservation: None,
            activation: None,
            task: None,
            scope: None,
            directory_completed: false,
            writer: None,
            #[cfg(test)]
            after_parts: None,
            #[cfg(test)]
            after_head: None,
            #[cfg(test)]
            after_pair: None,
            #[cfg(test)]
            after_temporary_refund: None,
            fee: None,
        }
    }

    fn require_layout_invoice() -> Result<(), AppError> {
        let bytes = V2GenesisRuntimeParts::<T, R>::allocation_bytes()
            .checked_add(std::mem::size_of::<Self>())
            .and_then(|n| n.checked_add(std::mem::size_of::<UnexposedV2GenesisCurrent<T, R>>()))
            .and_then(|n| n.checked_add(16 * 1024)) // scope attachment/return/control metadata
            .ok_or_else(critical_namespaces::refused)?;
        if bytes > WRAPPER_FEE {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    /// One finite trusted-local attempt. No automatic loop or new persistent execution grant.
    /// A failure retains this SAME source/task/nonce/candidates/owner/parts/node for retry.
    pub fn retry(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let store = layout.checked_store()?;
        self.graph.source()?.validate_original(store)?;
        Self::require_layout_invoice()?;
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
        // Temporary credit precedes even the first directory effect. It may be reacquired on
        // a later explicit attempt, but cannot reset any original persistent source allowance.
        let buffers = store.reserve_v2_genesis_scope_buffers()?;
        let result = (|| {
            // Another group's failed write can precede our first vacant admission. Save
            // only the already-authorized latest inventory; all own proofs still follow.
            store.resync_v2_current_scope_inventory(&buffers, rng)?;
            self.advance_storage(layout, store, &buffers, rng)?;
            self.pair_runtime(store)?;
            let source = self.graph.source()?;
            store.resync_v2_genesis_current(
                self.scope
                    .as_mut()
                    .ok_or_else(critical_namespaces::refused)?,
                &source,
                self.task
                    .as_ref()
                    .ok_or_else(critical_namespaces::refused)?,
                &buffers,
                rng,
            )
        })();
        // BOTH cleanup paths precede the final positive proof, including on errors. Their
        // account refunds can wait, so a prior source/head check cannot be reused afterward.
        drop(buffers);
        if let Some(writer) = &mut self.writer {
            writer.drain_unused_outside_writers();
        }
        result?;
        #[cfg(test)]
        if let Some(hook) = self.after_temporary_refund.take() {
            hook();
        }
        self.validate_ready(store)
    }

    fn advance_storage(
        &mut self,
        layout: &V2CurrentLayoutStore,
        store: &ServerStore,
        buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let source = self.graph.source()?;
        if self.activation.is_none() {
            self.activation = Some(PreparedV2GenesisActivation::prepare(store, &source, rng)?);
        }
        if self.task.is_none() {
            self.task = Some(layout.prepare_genesis_group_namespace(&source)?);
        }
        if !self.directory_completed {
            self.task
                .as_mut()
                .ok_or_else(critical_namespaces::refused)?
                .retry_directory(store, &source)?;
            self.directory_completed = true;
        }
        let task = self
            .task
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let activation = self
            .activation
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        if self.scope.is_none() {
            let seed = store.with_vacant_v2_genesis_admission(&source, |admission| {
                let guard = task.lock_vacant(admission)?;
                activation.save_seed(&guard)
            })?;
            // This method caches the exact Some1 preparation before the first row effect.
            store.reserve_v2_genesis_initializing(
                &source,
                task,
                seed,
                &mut self.scope,
                buffers,
                rng,
            )?;
        }
        let scope = self
            .scope
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        if self.writer.is_none() {
            self.writer = Some(PreparedV2WriterRegistration::prepare(
                store, &source, task, scope,
            )?);
        }
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(critical_namespaces::refused)?;
        #[cfg(test)]
        let after_head = &mut self.after_head;
        store.finish_v2_genesis_current(scope, &source, task, buffers, rng, |admission, _| {
            let saved = {
                let mut owner = task.lock_initializing_owner_namespace(admission)?;
                owner.install_writer(writer)?;
                let guard = owner.lock_archive_root_for_writer(writer)?;
                activation.save_head(&guard)?
            };
            #[cfg(test)]
            if let Some(hook) = after_head.take() {
                hook(store);
            }
            Ok(saved)
        })
    }

    fn pair_runtime(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if matches!(self.graph, Graph::Node { .. }) {
            return Ok(());
        }
        if let Some(origin) = self.warm_origin.as_ref() {
            // Genuine storage/head/writer proof BEFORE allocating the alias.
            // Unlike validate_ready this accepts the original still-unbound Seed;
            // it cannot extract a Node or issue positive operational authority.
            self.validate_storage_ready(store)?;
            let scope = self
                .scope
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?;
            let source = self.graph.source()?;
            if self.warm_reservation.is_none() {
                self.warm_reservation = Some(origin.prepare_current(scope, &source)?);
            }
            self.warm_reservation
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?
                .validate_current(origin, scope, &source)?;
            // Allocations/authority observations above can wait. Reprove the
            // SAME real Current association before entering the SDK attachment.
            self.validate_storage_ready(store)?;
        }
        let scope = self
            .scope
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let Graph::Seed(seed) = &mut self.graph else {
            return Err(critical_namespaces::refused());
        };
        // Scope/authority attachment can allocate or wait. It is outside ALL host writers,
        // and the later node-derived full Current proof must follow every such callback.
        if let Some(warm) = self.warm_reservation.as_ref() {
            seed.seed.prepare_scope(warm)?;
        } else {
            seed.seed.prepare_scope(scope)?;
        }
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
        #[cfg(test)]
        if let Some(hook) = self.after_parts.take() {
            hook();
        }
        // End the mutable seed loan BEFORE taking it. Pairing itself allocates nothing and
        // its owning refusal returns both EXACT same owners, including their R/Clock/T.
        let Graph::Seed(seed) = std::mem::replace(&mut self.graph, Graph::Moving) else {
            unreachable!("checked seed phase");
        };
        let RetainedBootstrapGenesisSeed { seed, resources } = seed;
        let parts = self.parts.take().expect("cached paid runtime parts");
        match seed.prepare_runtime(parts) {
            Ok(prepared) => {
                self.graph = Graph::Node {
                    node: prepared.into_node(),
                    resources,
                };
                #[cfg(test)]
                if let Some(hook) = self.after_pair.take() {
                    hook();
                }
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
        if self.warm_origin.is_some() && self.warm_reservation.is_none() {
            return Err(critical_namespaces::refused());
        }
        self.validate_storage_ready(store)
    }

    /// Actual source/head/task/writer proof shared by the warm pre-pair cut and
    /// the final Node gate. This never relaxes validate_ready's Node requirement.
    fn validate_storage_ready(&self, store: &ServerStore) -> Result<(), AppError> {
        let scope = self
            .scope
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let head = scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        // These are the exact cached owners that produced this head, not whichever writer
        // or artifact happens to have matching data. All resource waits/full proofs follow.
        self.activation
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
        let source = self.graph.source()?;
        if let (Some(origin), Some(warm)) = (&self.warm_origin, &self.warm_reservation) {
            warm.validate_current(origin, scope, &source)?;
        }
        store.validate_v2_genesis_current(
            scope,
            &source,
            self.task
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?,
        )
    }

    /// Validate live original Current2 again at extraction. A cached success/ready flag is
    /// insufficient; a dirty unrelated row or revoked/replaced original preserves this owner.
    pub fn take_current(
        &mut self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<Option<UnexposedV2GenesisCurrent<T, R>>, AppError> {
        if !matches!(self.graph, Graph::Node { .. }) {
            return Ok(None);
        }
        if self.activation.is_none() || self.writer.is_none() {
            return Err(critical_namespaces::refused());
        }
        self.validate_ready(layout.checked_store()?)?;
        // Only infallible moves after the final physical/scope/source proof. No refunds,
        // serializer, allocator, SDK/authority callback or pending metadata cleanup follows.
        let Graph::Node { node, resources } = std::mem::replace(&mut self.graph, Graph::Moving)
        else {
            unreachable!("validated node phase");
        };
        Ok(Some(UnexposedV2GenesisCurrent {
            node,
            resources,
            activation: self.activation.take().expect("validated activation"),
            task: self.task.take().expect("validated task"),
            scope: self.scope.take().expect("validated scope"),
            writer: self.writer.take().expect("validated writer"),
            seed_settlement: None,
            fee: self.fee.take().expect("validated wrapper account"),
        }))
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
impl<T: MeshTransport, R: CryptoRngCore> PreparedV2GenesisCurrent<T, R> {
    /// Exercise the actual installed node's ordinary-member source predicate,
    /// not merely the adapter's copy of the cancellation observation.
    pub(in crate::store) fn warm_member_source_for_test(
        &self,
    ) -> Result<[u8; 32], catcoms_sync::owner_v2::V2MemberSignRefusal> {
        match &self.graph {
            Graph::Node { node, .. } => node.ordinary_member_signer_data(),
            _ => Err(catcoms_sync::owner_v2::V2MemberSignRefusal::Source),
        }
    }
    /// Read-only negatives from the REAL original/Current bridge; no fixture authority.
    pub(in crate::store) fn warm_negatives_for_test(
        &self,
    ) -> Option<(
        catcoms_rt::RequestCancellation,
        catcoms_rt::RequestCancellation,
        catcoms_rt::RequestCancellation,
    )> {
        use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
        Some((
            self.warm_origin.as_ref()?.negative_for_test(),
            self.scope.as_ref()?.reservation_observer(),
            self.warm_reservation.as_ref()?.reservation_observer(),
        ))
    }
    pub(in crate::store) fn cancel_warm_current_for_test(&self) {
        self.scope
            .as_ref()
            .expect("genuine Current")
            .cancel_original();
    }
    /// Negative corruption only: it cannot produce a Current or make an owner live.
    pub(in crate::store) fn corrupt_warm_identity_for_test(&mut self) {
        self.warm_reservation
            .as_mut()
            .expect("genuine bridge")
            .corrupt_current_identity_for_test();
    }
}

impl<T, R> UnexposedV2GenesisCurrent<T, R> {
    fn source(&self) -> GenesisSourceDataRef<'_> {
        GenesisSourceDataRef {
            seed: self.node.borrow_genesis_seed_data(),
            resources: &self.resources,
        }
    }
    /// Borrowed verified genesis DATA, not a native disclosure/authoring or runnable node API.
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.source().data()
    }

    pub fn validate_original(&self, layout: &V2CurrentLayoutStore) -> Result<(), AppError> {
        if let Some(result) = settlement::validate_after_cleanup(
            &self.settlement_owner_ref()?,
            self.seed_settlement.as_ref(),
            layout,
        ) {
            return result;
        }
        let store = layout.checked_store()?;
        let head = self
            .scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        self.activation.require_same_saved_head(head)?;
        self.writer
            .require_same_observation(head.writer_observation())?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.fee.validate(pool, WRAPPER_FEE)?;
        store.validate_v2_genesis_current(&self.scope, &self.source(), &self.task)
    }
}

#[cfg(test)]
mod tests;
// Declare after opaque_debug so the private child sees that lexical macro. Only the two
// inert owning wrappers escape; all receipt/guard/registration constructors stay private.
mod operational;
pub(in crate::store) use operational::{
    OriginalV2TextResidentObservation, OriginalV2UnsignedPlanObservation,
    PreparedV2NextUnsignedAppend, PreparedV2TextResidentProjection,
    PreparedV2UnsignedOriginalCustody, V2AdvancedAppendBasis, V2TextResidentParentData,
};
mod text;
pub(in crate::store) use operational::negative::{
    PreparedV2TextSourceObserver, V2TextSourceNegativeObserver, V2_TEXT_SOURCE_OBSERVER_BYTES,
};
pub(in crate::store) use operational::owner::{V2TextOwnedCurrent, V2TextOwnerRef};
#[cfg(all(test, any(windows, target_os = "linux")))]
use operational::{
    arm_member_close_during_sign_for_test, arm_owner_cancel_during_sign_for_test,
    member_close_drained_during_sign_for_test, member_sign_dispatches_for_test,
};
pub use operational::{
    MemberSaveJob, MemberSaveJobReport, MemberSaveNativeAdmission, PreparedV2TextHost,
    PreparedV2TextRead, V2MemberDependency, V2MemberSaveOutcome, V2MemberSaveRefusal,
    V2MemberSignRequest, V2TextHostPolicy,
};
mod cold;
pub(super) mod settlement;
pub use cold::{PreparedColdV2GenesisCurrent, UnexposedColdV2GenesisCurrent};

mod initializing;
pub use initializing::{
    PreparedColdV2GenesisInitializing, PreparedColdV2GenesisPreInitializing,
    UnexposedRepairedV2GenesisCurrent,
};
