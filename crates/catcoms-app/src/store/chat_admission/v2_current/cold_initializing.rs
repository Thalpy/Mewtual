//! NEW cold Initializing1 owner and exact successor, never reconstructed warm Saved/Fresh.
//! Paired with the actual cold capture/task/kernel/IO/caller and genuine recovery tests.
//! A complete journal, source label or clean flag alone creates no owner.
use super::*;
use crate::store::persistence::v2_genesis_activation::initializing_repair::{
    ColdV2GenesisRepairSeed, SavedColdV2GenesisRepairedHead,
};

/// Same actual newly mounted owner through original row1 -> repaired Head -> row2. Runtime
/// owns its strong revocation Life/sender; this preparation keeps only funded weak handles.
pub(in crate::store) struct ColdV2InitializingPreparation {
    inner: ChatAdmissionPreparation,
    group_bounds: ChatAdmissionGroupBounds,
    task: OriginalV2GenesisGroupObservation,
    basis: ColdV2GenesisScopeBasis,
    progress: Progress,
    // These final resource fields drain after owned metadata. For an ordinary cold capture
    // this Seed payment funds the scope Life and both observer tails. A converted birth
    // additionally retains the SAME small original owner payment in the field below.
    seed: Arc<ColdV2GenesisRepairSeed>,
    // An actual no-row birth moves its SAME small owner payment here. Ordinary mounted
    // row1 capture leaves this None and retains its independently paid Seed as before.
    _born_owner_resources: Option<Arc<super::cold_preinitializing::BornScopeOwnerResources>>,
}
// Inline binding storage is charged before any writer. Boxing a successor would add a
// fallible allocation between the physical Head barrier and its irreversible scope latch.
#[allow(clippy::large_enum_variant)]
enum Progress {
    Initializing,
    HeadSavedPendingCurrent2(Arc<SavedColdV2GenesisRepairedHead>),
    CurrentBound {
        head: Arc<SavedColdV2GenesisRepairedHead>,
        binding: V2CurrentBinding,
    },
}

impl std::fmt::Debug for ColdV2InitializingPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdV2InitializingPreparation")
            .finish_non_exhaustive()
    }
}

/// Added to Root's NEW repair invoice, not taken from decoded saved sizes. Source/provider/
/// SDK/raw graph, existing scope-map/index, writer generation/backing and global scope buffer
/// have separate actual payments. No lifetime group or execution grant follows from this.
pub(in crate::store) const COLD_INITIALIZING_SCOPE_METADATA_BYTES: usize =
    std::mem::size_of::<ColdV2InitializingPreparation>()
        + std::mem::size_of::<Progress>()
        + std::mem::size_of::<AdmissionOwner>()
        + std::mem::size_of::<revocation::Owner>()
        + 2 * std::mem::size_of::<AdmissionAuthority>()
        + std::mem::size_of::<ColdV2GenesisScopeBasis>()
        + std::mem::size_of::<V2CurrentBinding>()
        + std::mem::size_of::<DurableColdGenesisInitializingAdmission<'static>>()
        + std::mem::size_of::<ColdInitializingGenesisHeadAdmission<'static>>()
        + std::mem::size_of::<ColdRepairedGenesisCurrentProofAdmission<'static>>()
        + std::mem::size_of::<ColdRepairedGenesisCurrentAdmission<'static>>()
        + 2 * GROUP_BOUNDS_ALLOCATION_BYTES
        + 3 * 256
        + 16 * 1024;

impl ManagedChatScopeReservation for ColdV2InitializingPreparation {
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

impl ColdV2InitializingPreparation {
    /// Private family-only, infallible field moves AFTER the concrete born converter's
    /// actual row/owner/claim/Root proofs. No raw key/Record factory is exposed to Store.
    pub(super) fn from_born_parts(
        inner: ChatAdmissionPreparation,
        group_bounds: ChatAdmissionGroupBounds,
        task: OriginalV2GenesisGroupObservation,
        basis: ColdV2GenesisScopeBasis,
        seed: Arc<ColdV2GenesisRepairSeed>,
        resources: Arc<super::cold_preinitializing::BornScopeOwnerResources>,
    ) -> Self {
        Self {
            inner,
            group_bounds,
            task,
            basis,
            progress: Progress::Initializing,
            seed,
            _born_owner_resources: Some(resources),
        }
    }
    /// Borrow only this actual CLEAN bound Current2 under the SAME held Runtime. This
    /// cached owner/Head/binding check intentionally performs no mandatory-seed physical
    /// validation: an admitted cleanup may already have removed that original name.
    /// The distinct settlement body proves the complete phase-selected files separately.
    pub(super) fn borrow_current2_admission<'writer>(
        &'writer self,
        store: &'writer ServerStore,
        runtime: &'writer Runtime,
        source: &'writer GenesisSourceDataRef<'writer>,
        task: &'writer PreparedV2GenesisGroupNamespace,
    ) -> Result<ColdRepairedGenesisCurrentAdmission<'writer>, AppError> {
        let admission = ColdRepairedGenesisCurrentAdmission {
            original: owned(store, runtime, source, task, self),
        };
        admission.require_current_lineage_for_own_group_task(task)?;
        Ok(admission)
    }

    pub(in crate::store) fn seed(&self) -> &Arc<ColdV2GenesisRepairSeed> {
        &self.seed
    }
    pub(in crate::store) fn cancel_original(&self) {
        ManagedChatScopeRevocation::revoke(&self.inner.revoker);
    }
    pub(in crate::store) fn saved_head(&self) -> Option<&Arc<SavedColdV2GenesisRepairedHead>> {
        match &self.progress {
            Progress::Initializing => None,
            Progress::HeadSavedPendingCurrent2(head) | Progress::CurrentBound { head, .. } => {
                Some(head)
            }
        }
    }
    pub(in crate::store) fn current_binding(&self) -> Result<&V2CurrentBinding, AppError> {
        match &self.progress {
            Progress::CurrentBound { binding, .. } => Ok(binding),
            _ => Err(refused()),
        }
    }

    /// Cached association only. Deliberately no quota/root/SDK/encoding/file work and no
    /// cleanliness requirement; the exact clean effect/read constructors add their gates.
    pub(in crate::store) fn require_original_source_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        require_source_pair(store, source)?;
        self.seed.retains_original(store, source)?;
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        let p = &self.inner;
        let context = self.seed.expectation_data().context.spec_data();
        if p.mount.is_cancelled()
            || p.admission.is_cancelled()
            || !Arc::ptr_eq(&p.mount_identity, &store.chat_mount.identity)
            || p.group() != context.group
            || p.signer() != &context.signer
            || p.domain != context.scope_domain
            || p.record.nonce != context.scope_nonce
            || p.record.origin != Origin::Fresh
            || p.record.provenance != context.fresh_attempt
            || p.record.phase != Phase::Initializing
            || p.record.revision != 1
        {
            return Err(refused());
        }
        self.basis.require_same_row(&p.key, &p.record)
    }
}

fn validate_seed_read(
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
    seed: &ColdV2GenesisRepairSeed,
) -> Result<(), AppError> {
    seed.retains_original(store, source)?;
    let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
    seed.validate_under_archive_root(store, source, root.as_ref().ok_or_else(refused)?)
}

fn unowned_initializing_row(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    key: &ChatAdmissionKey,
    bounds: &ChatAdmissionGroupBounds,
    seed: &ColdV2GenesisRepairSeed,
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
    let context = seed.expectation_data().context.spec_data();
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(key)
        .ok_or_else(refused)?;
    if key.group() != context.group
        || key.signer() != &context.signer
        || row.phase != Phase::Initializing
        || row.revision != 1
        || row.origin != Origin::Fresh
        || row.provenance != context.fresh_attempt
        || row.nonce != context.scope_nonce
        || context.planned_revision != 1
    {
        return Err(refused());
    }
    seed.scope_basis_data().require_same_row(key, row)?;
    Ok(row.clone())
}

fn require_owned_row(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &ColdV2InitializingPreparation,
) -> Result<(), AppError> {
    require_base(store, runtime)?;
    preparation.require_original_source_negative(store, source, task)?;
    let p = &preparation.inner;
    preparation
        .group_bounds
        .require_no_foreign(runtime, &p.key)?;
    if runtime.predecessors.excludes_group(p.group()) {
        return Err(refused());
    }
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(&p.key)
        .ok_or_else(refused)?;
    let owner = runtime.owners.get(&p.key).ok_or_else(refused)?;
    // The original basis stays IMMUTABLE row1. Only this actual cached repaired Head can
    // justify its exact row2 successor; never update/recapture the basis from decoded data.
    let valid_phase = (row.phase == Phase::Initializing && row.revision == 1)
        || (row.phase == Phase::Current && row.revision == 2 && preparation.saved_head().is_some());
    if !valid_phase
        || row.nonce != p.record.nonce
        || row.origin != p.record.origin
        || row.provenance != p.record.provenance
        || owner.nonce != row.nonce
        || owner.revision != row.revision
        || !owner.life.live()
        || !Arc::ptr_eq(&owner.life.identity(), &p.owner_identity)
    {
        return Err(refused());
    }
    Ok(())
}

impl ServerStore {
    /// Cache the actual new row1 owner BEFORE Runtime insertion, without changing the
    /// authenticated row, nonce/provenance, source counter or any SDK state. Caller retains
    /// SAME slot/seed/task on every refusal. No generic vacancy/initializer is invoked.
    pub(in crate::store) fn prepare_cold_v2_initializing_scope(
        &self,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        seed: Arc<ColdV2GenesisRepairSeed>,
        slot: &mut Option<ColdV2InitializingPreparation>,
    ) -> Result<(), AppError> {
        seed.retains_original(self, source)?;
        source.validate_original(self)?;
        // This strong real receipt pays every key/watch/bounds on every error path. It
        // was acquired outside writers and drains after the function-local Runtime guard.
        let context = seed.expectation_data().context.spec_data();
        let key = ChatAdmissionKey::new(context.group, context.signer)?;
        let bounds = if slot.is_none() {
            Some(ChatAdmissionGroupBounds::new(context.group)?)
        } else {
            None
        };
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(key.group())?;
        if slot.is_none() {
            task.require_registered_cold_initializing_claim(self, source, &seed)?;
            let row = unowned_initializing_row(
                self,
                &runtime,
                source,
                &key,
                bounds.as_ref().ok_or_else(refused)?,
                &seed,
            )?;
            validate_seed_read(self, source, &seed)?;
            let owner_key = key.clone();
            let (inner, owner) = self.new_reservation_with_custody(&key, &row, seed.clone())?;
            // Allocations/account/file checks precede the final unowned-row/source test.
            // There is no fallible/drop gap between actual cache and owner insertion.
            validate_seed_read(self, source, &seed)?;
            task.require_registered_cold_initializing_claim(self, source, &seed)?;
            let now = unowned_initializing_row(
                self,
                &runtime,
                source,
                &key,
                bounds.as_ref().ok_or_else(refused)?,
                &seed,
            )?;
            if row != now || inner.mount.is_cancelled() || inner.admission.is_cancelled() {
                return Err(refused());
            }
            *slot = Some(ColdV2InitializingPreparation {
                inner,
                group_bounds: bounds.expect("allocated before Runtime"),
                task: task.observe_original(),
                basis: seed.scope_basis_data().clone(),
                progress: Progress::Initializing,
                seed: seed.clone(),
                _born_owner_resources: None,
            });
            runtime.owners.insert(owner_key, owner);
        }
        let preparation = slot.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(preparation.seed(), &seed) {
            return Err(refused());
        }
        require_owned_row(self, &runtime, source, task, preparation)?;
        let admission = owned(self, &runtime, source, task, preparation);
        task.require_cold_initializing_scope_owner(&admission)
    }
}

/// Same owned Runtime for cached identity/negative checks. A dirty table is visible here
/// only negatively; it grants no Init1 storage, writer install or Current2 binding.
pub(in crate::store) struct ColdInitializingGenesisAdmission<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2InitializingPreparation,
    _not_send: PhantomData<Rc<()>>,
}
fn owned<'writer>(
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2InitializingPreparation,
) -> ColdInitializingGenesisAdmission<'writer> {
    ColdInitializingGenesisAdmission {
        store,
        runtime,
        source,
        task,
        preparation,
        _not_send: PhantomData,
    }
}

impl ColdInitializingGenesisAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.source
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.task
    }
    pub(in crate::store) fn seed(&self) -> &Arc<ColdV2GenesisRepairSeed> {
        &self.preparation.seed
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
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        require_owned_row(
            self.store,
            self.runtime,
            self.source,
            self.task,
            self.preparation,
        )
    }
    pub(in crate::store) fn require_initializing_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.validate_source_only()?;
        if self.runtime.dirty {
            return Err(refused());
        }
        let row = self
            .runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&self.preparation.inner.key)
            .ok_or_else(refused)?;
        if row.phase != Phase::Initializing || row.revision != 1 {
            return Err(refused());
        }
        self.preparation.task.require_same_original(task)?;
        task.require_cold_initializing_scope_owner(self)
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

/// The actual row1 file AND vault-parent barrier have completed in this invocation while
/// the SAME admission writer remains held. A loaded clean flag, cached seed or old barrier
/// cannot construct this role. It alone is accepted by the new cold repair writer.
pub(in crate::store) struct DurableColdGenesisInitializingAdmission<'writer> {
    original: ColdInitializingGenesisAdmission<'writer>,
    permit: ChatAdmissionStoragePermit<'writer>,
}

/// Existing complete repaired Head on row1. This is read-only and never sufficient to
/// install a writer or modify pin/stage/current; it serves a lost repair return/revision cut.
pub(in crate::store) struct ColdInitializingGenesisHeadAdmission<'writer> {
    original: ColdInitializingGenesisAdmission<'writer>,
}

/// Same owner and latched repaired Head on row2, possibly dirty. This proof role exposes
/// no installer/repair callback and cannot be used to recreate a row1 storage permit.
pub(in crate::store) struct ColdRepairedGenesisCurrentProofAdmission<'writer> {
    original: ColdInitializingGenesisAdmission<'writer>,
}

/// Final CLEAN row2 plus actual original binding. Temporary serialization credits and
/// retired registry owners must have drained before the owning caller invokes this lane.
pub(in crate::store) struct ColdRepairedGenesisCurrentAdmission<'writer> {
    original: ColdInitializingGenesisAdmission<'writer>,
}

macro_rules! delegate_owned {
    ($ty:ident) => {
        impl<'writer> $ty<'writer> {
            /// Cached owner loan only. Concrete namespace entry points add their own exact
            /// phase/clean/Head gates; this loan cannot issue a durable Init1 role.
            pub(in crate::store) fn owner_admission(
                &self,
            ) -> &ColdInitializingGenesisAdmission<'writer> {
                &self.original
            }
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.original.store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.original.source()
            }
            pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
                self.original.task()
            }
            pub(in crate::store) fn seed(&self) -> &Arc<ColdV2GenesisRepairSeed> {
                self.original.seed()
            }
            pub(in crate::store) fn group(&self) -> &[u8] {
                self.original.group()
            }
            pub(in crate::store) fn signer(&self) -> &[u8; 32] {
                self.original.signer()
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.original.validate_source_only()
            }
            pub(in crate::store) fn current_filename_association(
                &self,
            ) -> Result<
                crate::store::chat_admission::current_lineage::CurrentFilenameAssociation<'_>,
                AppError,
            > {
                self.original.current_filename_association()
            }
        }
    };
}
delegate_owned!(DurableColdGenesisInitializingAdmission);
delegate_owned!(ColdInitializingGenesisHeadAdmission);
delegate_owned!(ColdRepairedGenesisCurrentProofAdmission);
delegate_owned!(ColdRepairedGenesisCurrentAdmission);

impl DurableColdGenesisInitializingAdmission<'_> {
    pub(in crate::store) fn scope_permit(&self) -> &ChatAdmissionStoragePermit<'_> {
        &self.permit
    }
    pub(in crate::store) fn require_initializing_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.original
            .require_initializing_lineage_for_own_group_task(task)?;
        self.permit
            .validate_original(self.store(), self.group(), self.signer())?;
        task.require_cold_initializing_owned_handoff(&self.original)
    }
}

macro_rules! retained_head {
    ($ty:ident) => {
        impl $ty<'_> {
            pub(in crate::store) fn head(&self) -> &Arc<SavedColdV2GenesisRepairedHead> {
                self.original
                    .preparation
                    .saved_head()
                    .expect("private retained Head role")
            }
        }
    };
}
retained_head!(ColdInitializingGenesisHeadAdmission);
retained_head!(ColdRepairedGenesisCurrentProofAdmission);
retained_head!(ColdRepairedGenesisCurrentAdmission);

impl ColdInitializingGenesisHeadAdmission<'_> {
    pub(in crate::store) fn require_initializing_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.original
            .require_initializing_lineage_for_own_group_task(task)?;
        self.head()
            .retains_original(self.store(), self.source(), task)?;
        task.require_cold_initializing_owned_handoff(&self.original)
    }
}
impl ColdRepairedGenesisCurrentProofAdmission<'_> {
    pub(in crate::store) fn require_retained_current_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        require_current2(&self.original)?;
        self.original.preparation.task.require_same_original(task)?;
        self.head()
            .retains_original(self.store(), self.source(), task)?;
        // This is a negative SAME-owner comparison, deliberately dirty tolerant. The
        // canonical proof must additionally match the original live installed generation.
        task.require_cold_initializing_owned_handoff(&self.original)
    }
}
impl ColdRepairedGenesisCurrentAdmission<'_> {
    pub(in crate::store) fn require_current_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        require_current2(&self.original)?;
        if self.original.runtime.dirty {
            return Err(refused());
        }
        self.original.preparation.task.require_same_original(task)?;
        require_binding(self.store(), self.original.preparation)?;
        self.head()
            .retains_original(self.store(), self.source(), task)?;
        task.require_cold_initializing_owned_handoff(&self.original)
    }
}

fn require_current2(admission: &ColdInitializingGenesisAdmission<'_>) -> Result<(), AppError> {
    admission.validate_source_only()?;
    let row = admission
        .runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(&admission.preparation.inner.key)
        .ok_or_else(refused)?;
    if row.phase != Phase::Current
        || row.revision != 2
        || admission.preparation.saved_head().is_none()
    {
        return Err(refused());
    }
    Ok(())
}

fn require_binding(
    store: &ServerStore,
    preparation: &ColdV2InitializingPreparation,
) -> Result<(), AppError> {
    let binding = preparation.current_binding()?;
    let p = &preparation.inner;
    if binding.inner.key != p.key
        || binding.inner.nonce != p.record.nonce
        || binding.inner.revision != 2
        || binding.inner.domain != p.domain
        || !Arc::ptr_eq(&binding.inner.owner_identity, &p.owner_identity)
        || !Arc::ptr_eq(&binding.inner.mount_identity, &store.chat_mount.identity)
        || binding.reservation_observer().is_cancelled()
    {
        return Err(refused());
    }
    Ok(())
}

/// Actual same-row1 proof. Never holds Root while writing the global scope inventory.
fn validate_initializing_head(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &ColdV2InitializingPreparation,
) -> Result<(), AppError> {
    let admission = ColdInitializingGenesisHeadAdmission {
        original: owned(store, runtime, source, task, preparation),
    };
    admission.require_initializing_lineage_for_own_group_task(task)?;
    let guard = task.lock_cold_initializing_head_read(&admission)?;
    admission.head().validate_under_initializing_guard(&guard)?;
    guard.validate_source_only()
}

fn validate_current2_head(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &ColdV2InitializingPreparation,
) -> Result<(), AppError> {
    let admission = ColdRepairedGenesisCurrentProofAdmission {
        original: owned(store, runtime, source, task, preparation),
    };
    admission.require_retained_current_lineage_for_own_group_task(task)?;
    let guard = task.lock_cold_repaired_current_proof(&admission)?;
    admission.head().validate_under_proof_guard(&guard)?;
    guard.validate_source_only()
}

impl ServerStore {
    /// Same actual owner borrower for the one-time pending-slot handoff. This function
    /// cannot perform repair/registration itself. Returned paid retired debt must drain
    /// OUTSIDE this callback and every admission/pins/canonical/Root guard.
    pub(in crate::store) fn with_cold_initializing_admission<T>(
        &self,
        preparation: &ColdV2InitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        callback: impl for<'writer> FnOnce(
            &ColdInitializingGenesisAdmission<'writer>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        let admission = owned(self, &runtime, source, task, preparation);
        admission.require_initializing_lineage_for_own_group_task(task)?;
        let result = callback(&admission)?;
        admission.validate_source_only()?;
        Ok(result)
    }

    /// NEW even-clean latest-table barrier, immediately followed by the concrete repair
    /// callback under the SAME AD. Original row1/nonce/owner never changes here. An error
    /// retains callback's own selected-file/SavedHead state; it does not certify Current2.
    // Keep actual owner/source/task and their independently paid scope buffers explicit.
    // Combining them into a generic context would obscure the distinct borrowed roles
    // at the barrier-to-repair handoff; this helper is private to the concrete finisher.
    #[allow(clippy::too_many_arguments)]
    fn with_durable_cold_genesis_initializing(
        &self,
        runtime: &mut Runtime,
        preparation: &ColdV2InitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
        repair: impl for<'writer> FnOnce(
            &DurableColdGenesisInitializingAdmission<'writer>,
        )
            -> Result<Arc<SavedColdV2GenesisRepairedHead>, AppError>,
    ) -> Result<Arc<SavedColdV2GenesisRepairedHead>, AppError> {
        require_buffers(self, scope_buffers)?;
        require_owned_row(self, runtime, source, task, preparation)?;
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&preparation.inner.key)
            .ok_or_else(refused)?;
        if row.phase != Phase::Initializing || row.revision != 1 {
            return Err(refused());
        }
        task.require_cold_initializing_owned_handoff(&owned(
            self,
            runtime,
            source,
            task,
            preparation,
        ))?;
        validate_seed_read(self, source, &preparation.seed)?;
        require_owned_row(self, runtime, source, task, preparation)?;
        // Explicitly EVEN CLEAN. This function also repairs already-admitted dirty table
        // debt, without a reload or dropping the same writer. No pins/Root guard is live.
        self.flush_chat_admissions(runtime, rng)?;
        require_buffers(self, scope_buffers)?;
        validate_seed_read(self, source, &preparation.seed)?;
        let original = owned(self, runtime, source, task, preparation);
        original.require_initializing_lineage_for_own_group_task(task)?;
        let durable = DurableColdGenesisInitializingAdmission {
            original,
            permit: ChatAdmissionStoragePermit::initializing(self, runtime, &preparation.inner),
        };
        durable.require_initializing_lineage_for_own_group_task(task)?;
        let head = repair(&durable)?;
        // Only exact cached negatives after the callback: Root has cached the actual Head
        // before any fallible return, and only that SAME receipt may advance the scope.
        durable.validate_source_only()?;
        Ok(head)
    }

    /// Repair once, retain its Head before row mutation, then advance the SAME owner1→2.
    /// No path at row2 can invoke repair, install another writer, or issue a row1 permit.
    pub(in crate::store) fn finish_cold_v2_initializing_current(
        &self,
        preparation: &mut ColdV2InitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
        repair: impl for<'writer> FnOnce(
            &DurableColdGenesisInitializingAdmission<'writer>,
        )
            -> Result<Arc<SavedColdV2GenesisRepairedHead>, AppError>,
    ) -> Result<(), AppError> {
        require_buffers(self, scope_buffers)?;
        preparation.require_original_source_negative(self, source, task)?;
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        require_owned_row(self, &runtime, source, task, preparation)?;
        let initializing = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&preparation.inner.key)
            .ok_or_else(refused)?
            .phase
            == Phase::Initializing;
        if initializing {
            if preparation.saved_head().is_none() {
                let head = self.with_durable_cold_genesis_initializing(
                    &mut runtime,
                    preparation,
                    source,
                    task,
                    scope_buffers,
                    rng,
                    repair,
                )?;
                if !Arc::ptr_eq(head.seed(), &preparation.seed) {
                    return Err(refused());
                }
                // Cache BEFORE any later full proof/row2 barrier/binder. A late negative
                // keeps terminal Head custody and cannot turn this attempt into repairable1.
                preparation.progress = Progress::HeadSavedPendingCurrent2(head);
            }
            if runtime.dirty {
                require_buffers(self, scope_buffers)?;
                validate_seed_read(self, source, &preparation.seed)?;
                require_owned_row(self, &runtime, source, task, preparation)?;
                self.flush_chat_admissions(&mut runtime, rng)?;
            }
            validate_initializing_head(self, &runtime, source, task, preparation)?;
            require_owned_row(self, &runtime, source, task, preparation)?;
            let old = runtime
                .table
                .as_ref()
                .ok_or_else(refused)?
                .get(&preparation.inner.key)
                .cloned()
                .ok_or_else(refused)?;
            if old.phase != Phase::Initializing || old.revision != 1 {
                return Err(refused());
            }
            runtime
                .predecessors
                .before_row_change(&preparation.inner.key, Some(&old));
            let row = runtime
                .table
                .as_mut()
                .ok_or_else(refused)?
                .get_mut(&preparation.inner.key)
                .ok_or_else(refused)?;
            row.phase = Phase::Current;
            row.revision = 2;
            // Same strong revocation Life/sender. reservation_locked would cancel on a
            // revision mismatch and is forbidden between the two matching updates.
            runtime
                .owners
                .get_mut(&preparation.inner.key)
                .ok_or_else(refused)?
                .revision = 2;
            runtime.dirty = true;
        }
        finish_current2(
            self,
            &mut runtime,
            source,
            task,
            preparation,
            scope_buffers,
            rng,
        )
    }

    /// Recheck/flush an ALREADY bound repaired Current2 after the graph moved into its
    /// private node. No repair callback, install or Init1 constructor exists in this lane.
    pub(in crate::store) fn resync_cold_repaired_v2_genesis_current(
        &self,
        preparation: &mut ColdV2InitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        preparation.current_binding()?;
        require_buffers(self, scope_buffers)?;
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        finish_current2(
            self,
            &mut runtime,
            source,
            task,
            preparation,
            scope_buffers,
            rng,
        )
    }

    pub(in crate::store) fn validate_cold_repaired_v2_genesis_current(
        &self,
        preparation: &ColdV2InitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        let admission = ColdRepairedGenesisCurrentAdmission {
            original: owned(self, &runtime, source, task, preparation),
        };
        admission.require_current_lineage_for_own_group_task(task)?;
        let guard = task.lock_cold_repaired_current(&admission)?;
        admission.head().validate_under_current_guard(&guard)?;
        admission.require_current_lineage_for_own_group_task(task)
    }
}

fn finish_current2(
    store: &ServerStore,
    runtime: &mut Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &mut ColdV2InitializingPreparation,
    scope_buffers: &CatalogReadReservation,
    rng: &mut impl CryptoRngCore,
) -> Result<(), AppError> {
    require_buffers(store, scope_buffers)?;
    validate_current2_head(store, runtime, source, task, preparation)?;
    if runtime.dirty {
        require_buffers(store, scope_buffers)?;
        validate_current2_head(store, runtime, source, task, preparation)?;
        store.flush_chat_admissions(runtime, rng)?;
    }
    validate_current2_head(store, runtime, source, task, preparation)?;
    if !matches!(&preparation.progress, Progress::CurrentBound { .. }) {
        let inner = store.bind_owned_chat_admission_locked(
            runtime,
            &preparation.inner.key,
            preparation.inner.nonce(),
            2,
            &preparation.inner.owner_identity,
        )?;
        let head = preparation.saved_head().ok_or_else(refused)?.clone();
        let binding = V2CurrentBinding {
            inner,
            writer: head.writer_observation().clone(),
        };
        // Binder/watch work may wait/allocate. Full proof follows; only same actual graph,
        // owner/writer/source negatives precede the exclusive sticky phase move.
        validate_current2_head(store, runtime, source, task, preparation)?;
        if binding.reservation_observer().is_cancelled() {
            return Err(refused());
        }
        preparation.progress = Progress::CurrentBound { head, binding };
    }
    require_binding(store, preparation)?;
    require_owned_row(store, runtime, source, task, preparation)
}
