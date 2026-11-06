//! Actual cold Current2 scope owner, jointly consumed by Factory's registered-claim/owner-
//! handoff roles and Root's cold canonical writer and owning library caller. The nested
//! private binding supplies no runnable node or native activation. No row rewrite,
//! head callback, fresh nonce or old warm proof exists here. Cold requires exact pin/head,
//! no stage and either its actual seed or the separate strict Windows absence/covering role.
use super::cold_read::ColdV2GenesisScopeBasis;
use super::*;
use crate::store::persistence::v2_genesis_activation::ColdV2GenesisCurrentHead;

#[path = "cold_binding.rs"]
mod binding;
pub(in crate::store) use binding::ColdCurrentProofAdmission;
use binding::ColdProgress;

/// Private mounted custody for an ALREADY durable Current2. Historical Some(1) matches the
/// original S0 plan; the actual immutable `inner.record` remains Current/revision2 throughout.
/// Runtime also owns the same real receipt payment, so caller Drop cannot refund its life.
pub(in crate::store) struct ColdV2CurrentPreparation {
    inner: ChatAdmissionPreparation,
    group_bounds: ChatAdmissionGroupBounds,
    task: OriginalV2GenesisGroupObservation,
    basis: ColdV2GenesisScopeBasis,
    progress: ColdProgress,
    // LAST after setup/key/weak observation metadata. This actual private receipt contains
    // no source graph, key, Store, session or sender; it funds Runtime and escaping tails too.
    head: Arc<ColdV2GenesisCurrentHead>,
}

/// Private process-local DATA for an exhaustive caller match. This selects which concrete
/// retry method to invoke; it grants no owner, physical proof, installed writer or binding.
/// Every selected method repeats its own typed phase and original-source checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum ColdV2CurrentProgressData {
    Unbound,
    WriterValidated,
    CurrentBound,
}
impl std::fmt::Debug for ColdV2CurrentPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdV2CurrentPreparation")
            .finish_non_exhaustive()
    }
}

/// Named logical setup/retention inside Root's actual cold receipt metadata debit. Its invoice
/// must assert this ABI before allocating its receipt. Existing global BTree nodes, raw
/// source/SDK/runtime payloads and temporary32MiB scope serialization are separate contracts.
pub(in crate::store) const COLD_V2_CURRENT_SCOPE_METADATA_BYTES: usize =
    std::mem::size_of::<ColdV2CurrentPreparation>()
        + std::mem::size_of::<ColdV2CurrentProgressData>()
        + std::mem::size_of::<ColdOwnedCurrentAdmission<'static>>()
        + std::mem::size_of::<ColdCurrentProofAdmission<'static>>()
        + std::mem::size_of::<V2CurrentBinding>()
        + std::mem::size_of::<AdmissionOwner>()
        + std::mem::size_of::<revocation::Owner>()
        + 2 * std::mem::size_of::<AdmissionAuthority>()
        + std::mem::size_of::<ColdV2GenesisScopeBasis>()
        + GROUP_BOUNDS_ALLOCATION_BYTES
        + 3 * 256
        + 16 * 1024;

impl ManagedChatScopeReservation for ColdV2CurrentPreparation {
    fn group(&self) -> &[u8] {
        self.inner.group()
    }
    fn signer(&self) -> &[u8; 32] {
        self.inner.signer()
    }
    fn domain(&self) -> [u8; 32] {
        self.inner.domain()
    }
    fn nonce(&self) -> [u8; 32] {
        self.inner.nonce()
    }
    fn planned_revision_data(&self) -> Option<u64> {
        Some(1)
    }
    fn owner_identity(&self) -> Arc<()> {
        self.inner.owner_identity()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.inner.reservation_observer()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        self.inner.revocation()
    }
}
impl ColdV2CurrentPreparation {
    pub(in crate::store) fn progress_data(&self) -> ColdV2CurrentProgressData {
        match &self.progress {
            ColdProgress::Unbound => ColdV2CurrentProgressData::Unbound,
            ColdProgress::WriterValidated(_) => ColdV2CurrentProgressData::WriterValidated,
            ColdProgress::CurrentBound { .. } => ColdV2CurrentProgressData::CurrentBound,
        }
    }
    pub(in crate::store) fn head(&self) -> &Arc<ColdV2GenesisCurrentHead> {
        &self.head
    }
    #[cfg(test)]
    pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
        &self.basis
    }
    pub(in crate::store) fn cancel_original(&self) {
        ManagedChatScopeRevocation::revoke(&self.inner.revoker);
    }

    /// Resource/identity negatives for the paid canonical writer preparation OUTSIDE
    /// writers. This is neither an installed writer, a new owner nor a physical proof.
    pub(in crate::store) fn require_original_source_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        require_source_pair(store, source)?;
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        self.head.retains_original(store, source)?;
        let context = self.head.expectation_data().context.spec_data();
        let p = &self.inner;
        if p.mount.is_cancelled()
            || p.admission.is_cancelled()
            || !Arc::ptr_eq(&p.mount_identity, &store.chat_mount.identity)
            || p.group() != context.group
            || p.signer() != &context.signer
            || p.domain != context.scope_domain
            || p.record.nonce != context.scope_nonce
            || p.record.provenance != context.fresh_attempt
            || p.record.origin != Origin::Fresh
            || p.record.phase != Phase::Current
            || p.record.revision != 2
        {
            return Err(refused());
        }
        self.basis.require_same_row(&p.key, &p.record)
    }
}

/// SAME real Runtime writer after this private owner was cached. This is not the unowned
/// cold-read admission or warm Init1 writer. A concrete Factory guard must additionally
/// validate its actual registered claim OR exact private OnceLock owner association.
pub(in crate::store) struct ColdOwnedCurrentAdmission<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2CurrentPreparation,
    _not_send: PhantomData<Rc<()>>,
}
impl std::fmt::Debug for ColdOwnedCurrentAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdOwnedCurrentAdmission")
            .finish_non_exhaustive()
    }
}
impl ColdOwnedCurrentAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.source
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.task
    }
    pub(in crate::store) fn head(&self) -> &Arc<ColdV2GenesisCurrentHead> {
        &self.preparation.head
    }
    pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
        &self.preparation.basis
    }
    pub(in crate::store) fn owner_identity(&self) -> &Arc<()> {
        &self.preparation.inner.owner_identity
    }
    pub(in crate::store) fn owner_observer(&self) -> &RequestCancellation {
        &self.preparation.inner.admission
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.preparation.inner.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.preparation.inner.signer()
    }
    pub(in crate::store) fn domain(&self) -> [u8; 32] {
        self.preparation.inner.domain()
    }
    pub(in crate::store) fn nonce(&self) -> [u8; 32] {
        self.preparation.inner.nonce()
    }
    pub(in crate::store) fn revision(&self) -> u64 {
        2
    }

    /// No lock/I/O/pool/SDK/callback; it deliberately does NOT call Factory's cached owner
    /// association helper, whose own checks use this predicate. Keep that call direction
    /// acyclic. The actual namespace constructor then conjoins its exact claim/handoff.
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        require_base(self.store, self.runtime)?;
        self.preparation
            .require_original_source_negative(self.store, self.source, self.task)?;
        self.preparation
            .group_bounds
            .require_no_foreign(self.runtime, &self.preparation.inner.key)?;
        if self.runtime.predecessors.excludes_group(self.group()) {
            return Err(refused());
        }
        let p = &self.preparation.inner;
        let row = self
            .runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&p.key)
            .ok_or_else(refused)?;
        let owner = self.runtime.owners.get(&p.key).ok_or_else(refused)?;
        if row != &p.record
            || owner.nonce != row.nonce
            || owner.revision != row.revision
            || !owner.life.live()
            || !Arc::ptr_eq(&owner.life.identity(), &p.owner_identity)
        {
            return Err(refused());
        }
        self.preparation.basis.require_same_row(&p.key, row)
    }
    pub(in crate::store) fn require_owned_current_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        // This concrete owner role can install/handoff an actual writer only against a
        // clean table. A later dirty-table resync must be its own funded scope consumer;
        // the cached negative source comparator below is not that permission.
        if self.runtime.dirty {
            return Err(refused());
        }
        self.preparation.task.require_same_original(task)?;
        self.validate_source_only()?;
        task.require_cold_current_scope_owner(self)
    }
    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<
        crate::store::chat_admission::current_lineage::CurrentFilenameAssociation<'_>,
        AppError,
    > {
        self.validate_source_only()?;
        self.runtime.current_filename_association(self.store)
    }
}

fn validate_cold_head_read(
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
    head: &ColdV2GenesisCurrentHead,
) -> Result<(), AppError> {
    head.retains_original(store, source)?;
    let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
    head.validate_under_archive_root(store, source, root.as_ref().ok_or_else(refused)?)
}

fn unowned_current_row(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    key: &ChatAdmissionKey,
    bounds: &ChatAdmissionGroupBounds,
    head: &ColdV2GenesisCurrentHead,
) -> Result<Record, AppError> {
    require_base(store, runtime)?;
    require_source_pair(store, source)?;
    bounds.require_no_foreign(runtime, key)?;
    if runtime.dirty
        || runtime.owners.contains_key(key)
        || runtime.predecessors.excludes_group(key.group())
    {
        return Err(refused());
    }
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(key)
        .ok_or_else(refused)?;
    let context = head.expectation_data().context.spec_data();
    if key.group() != context.group
        || key.signer() != &context.signer
        || row.phase != Phase::Current
        || row.revision != 2
        || row.origin != Origin::Fresh
        || row.nonce != context.scope_nonce
        || row.provenance != context.fresh_attempt
    {
        return Err(refused());
    }
    head.scope_basis_data().require_same_row(key, row)?;
    Ok(row.clone())
}

impl ServerStore {
    /// Cold owner-only transfer. Actual first registered task is required BEFORE cache and
    /// owner insertion. Existing Current2 is never rewritten or represented as warm Init1.
    /// Caller retains `slot` on every error; source/receipt originals are never reconstructed.
    pub(in crate::store) fn prepare_cold_v2_current_scope(
        &self,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        head: Arc<ColdV2GenesisCurrentHead>,
        slot: &mut Option<ColdV2CurrentPreparation>,
    ) -> Result<(), AppError> {
        head.retains_original(self, source)?;
        source.validate_original(self)?;
        let context = head.expectation_data().context.spec_data();
        let key = ChatAdmissionKey::new(context.group, context.signer)?;
        let bounds = if slot.is_none() {
            Some(ChatAdmissionGroupBounds::new(context.group)?)
        } else {
            None
        };
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(key.group())?;
        if slot.is_none() {
            task.require_registered_cold_current_claim(self, source, &head)?;
            let row = unowned_current_row(
                self,
                &runtime,
                source,
                &key,
                bounds.as_ref().ok_or_else(refused)?,
                &head,
            )?;
            validate_cold_head_read(self, source, &head)?;
            let owner_key = key.clone();
            let (inner, owner) = self.new_reservation_with_custody(&key, &row, head.clone())?;
            // Setup can allocate/watch/clone. Repeat the actual original physical closure
            // and all unowned/source/task/basis predicates AFTER it and BEFORE the cache.
            validate_cold_head_read(self, source, &head)?;
            task.require_registered_cold_current_claim(self, source, &head)?;
            let current = unowned_current_row(
                self,
                &runtime,
                source,
                &key,
                bounds.as_ref().ok_or_else(refused)?,
                &head,
            )?;
            if current != row || inner.mount.is_cancelled() || inner.admission.is_cancelled() {
                return Err(refused());
            }
            let preparation = ColdV2CurrentPreparation {
                inner,
                group_bounds: bounds.ok_or_else(refused)?,
                task: task.observe_original(),
                basis: head.scope_basis_data().clone(),
                progress: ColdProgress::Unbound,
                head: head.clone(),
            };
            *slot = Some(preparation);
            // No recoverable fallible work/callback here. BTree insertion retains its
            // ordinary infallible/OOM-abort allocation behavior, not a zero-allocation claim.
            runtime.owners.insert(owner_key, owner);
        }
        let preparation = slot.as_ref().ok_or_else(refused)?;
        if preparation.inner.key != key || !Arc::ptr_eq(&preparation.head, &head) {
            return Err(refused());
        }
        // Some retries after handoff validate the SAME owner association, not a retired
        // registry slot. The actual receipt pointer/read/SDK pairing is fixed by the prep.
        validate_cold_head_read(self, source, &preparation.head)?;
        let admission = ColdOwnedCurrentAdmission {
            store: self,
            runtime: &runtime,
            source,
            task,
            preparation,
            _not_send: PhantomData,
        };
        admission.require_owned_current_lineage_for_own_group_task(task)
    }

    /// The private HRTB callback can acquire concrete Factory cold owner guards/handoff or
    /// Root's distinct canonical current-read writer. It cannot return this Runtime borrow.
    pub(in crate::store) fn with_cold_owned_current_admission<T>(
        &self,
        preparation: &ColdV2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        callback: impl for<'writer> FnOnce(&ColdOwnedCurrentAdmission<'writer>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        preparation.require_original_source_negative(self, source, task)?;
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        let admission = ColdOwnedCurrentAdmission {
            store: self,
            runtime: &runtime,
            source,
            task,
            preparation,
            _not_send: PhantomData,
        };
        admission.require_owned_current_lineage_for_own_group_task(task)?;
        callback(&admission)
    }
}
