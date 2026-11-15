//! Complete authenticated activation seed with actual group-wide RAM row absence.
//! This is not generic vacancy/Fresh and never repairs a torn/missing activation seed.
//! Declare only with the actual Root capture, shared16 task, birth/transition and caller.
use super::*;
use crate::store::persistence::v2_genesis_activation::pre_initializing::{
    BornV2GenesisRepairSeed, ColdV2PreInitializingSeed,
};
use catcoms_sync::owner_v2::BootstrapSourceId;
use std::sync::OnceLock;

const MAX_GROUP: usize = 256;

/// Immutable DATA from a concrete original loaded-vacancy check. This is deliberately NOT
/// ColdV2GenesisScopeBasis: absence contains no actual row, phase or durable revision.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct ColdV2GenesisVacancyBasis {
    group: [u8; MAX_GROUP],
    length: u16,
    signer: [u8; 32],
    source: BootstrapSourceId,
}
impl std::fmt::Debug for ColdV2GenesisVacancyBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdV2GenesisVacancyBasis")
            .finish_non_exhaustive()
    }
}
impl ColdV2GenesisVacancyBasis {
    fn capture(
        source: &GenesisSourceDataRef<'_>,
        key: &ChatAdmissionKey,
    ) -> Result<Self, AppError> {
        if key.group().is_empty() || key.group().len() > MAX_GROUP {
            return Err(refused());
        }
        let mut group = [0; MAX_GROUP];
        group[..key.group().len()].copy_from_slice(key.group());
        Ok(Self {
            group,
            length: u16::try_from(key.group().len()).map_err(|_| refused())?,
            signer: *key.signer(),
            source: source.data().source,
        })
    }
    fn require_same_source(
        &self,
        source: &GenesisSourceDataRef<'_>,
        key: &ChatAdmissionKey,
    ) -> Result<(), AppError> {
        if self == &Self::capture(source, key)? {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

/// Pure paid accounting, NEVER an owner/handle/observer/Authority pointing back through
/// its own keepalive. The same Arc pays Runtime's strong Life and both weak observer tails;
/// it contains no source/SDK graph, large pre-seed receipt, task sender, Store or session.
pub(super) struct BornScopeOwnerResources {
    fee: CatalogReadReservation,
}
// The watch's blanket Debug + Send + Sync keepalive contract retains this
// accounting-only payment through its final observer tail. It grants no authority.
impl std::fmt::Debug for BornScopeOwnerResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BornScopeOwnerResources")
            .finish_non_exhaustive()
    }
}
const BORN_SCOPE_OWNER_FEE: usize = 64 * 1024;

pub(in crate::store) struct ColdV2PreInitializingPreparation {
    inner: ChatAdmissionPreparation,
    bounds: ChatAdmissionGroupBounds,
    task: OriginalV2GenesisGroupObservation,
    vacancy: ColdV2GenesisVacancyBasis,
    origin: Arc<ColdV2PreInitializingSeed>,
    progress: BirthProgress,
    // LAST. Independent small lifetime payment, not the large physical pre-seed class.
    owner_resources: Arc<BornScopeOwnerResources>,
}
enum BirthProgress {
    // The actual new preparation is cached before RAM owner/index/table transfer. A failed
    // scope write retains the SAME owned row1; retry MUST NOT perform a vacancy check.
    Row1Pending,
    // Actual Root result is cached before handoff/conversion can return or lose an error.
    RepairSeedSaved(Arc<BornV2GenesisRepairSeed>),
}
impl std::fmt::Debug for ColdV2PreInitializingPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdV2PreInitializingPreparation")
            .finish_non_exhaustive()
    }
}

/// Returned old strong DATA only. A caller MUST cache/drain it after AD/pins/canonical/Root
/// have all released. It owns no cancellation/revoker/sender that could close moved Current.
#[must_use = "drain retired birth metadata outside every host/SDK writer"]
pub(in crate::store) struct RetiredBornScopeCustody {
    _origin: Arc<ColdV2PreInitializingSeed>,
    _receipt: Arc<BornV2GenesisRepairSeed>,
}

/// Actual unowned group absence under this SAME loaded Runtime. Fixed metadata is paid
/// before keys/guards, and the !Send loan cannot escape its HRTB admission callback.
pub(in crate::store) struct ColdV2GenesisVacantReadAdmission<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    key: &'writer ChatAdmissionKey,
    bounds: &'writer ChatAdmissionGroupBounds,
    basis: ColdV2GenesisVacancyBasis,
    _not_send: PhantomData<Rc<()>>,
}
impl ColdV2GenesisVacantReadAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.source
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.key.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.key.signer()
    }
    pub(in crate::store) fn vacancy_basis_data(&self) -> &ColdV2GenesisVacancyBasis {
        &self.basis
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        require_base(self.store, self.runtime)?;
        require_source_pair(self.store, self.source)?;
        self.bounds.require_absent(self.runtime)?;
        if self.runtime.predecessors.excludes_group(self.group()) {
            return Err(refused());
        }
        self.basis.require_same_source(self.source, self.key)
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
const VACANCY_READ_METADATA_BYTES: usize = GROUP_BOUNDS_ALLOCATION_BYTES
    + MAX_GROUP
    + std::mem::size_of::<ChatAdmissionKey>()
    + 2 * std::mem::size_of::<ColdV2GenesisVacancyBasis>()
    + std::mem::size_of::<ColdV2GenesisVacantReadAdmission<'static>>()
    + 512;

impl ServerStore {
    /// Task-free existing-journal capture only. No table/owner, nonce, SourceIntent, SDK or
    /// generic vacant/mkdir operation is performed. Source+SDK were independently paid.
    pub(in crate::store) fn with_cold_v2_genesis_vacant_read<T>(
        &self,
        source: &GenesisSourceDataRef<'_>,
        callback: impl for<'writer> FnOnce(
            &ColdV2GenesisVacantReadAdmission<'writer>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        require_source_pair(self, source)?;
        let pool = &self
            .current_layout_policy
            .as_ref()
            .ok_or_else(refused)?
            .archive
            .pending
            .pool;
        let fee = pool.try_reserve(VACANCY_READ_METADATA_BYTES)?;
        let context = source.data().context.spec_data();
        let key = ChatAdmissionKey::new(context.group, context.signer)?;
        let bounds = ChatAdmissionGroupBounds::new(context.group)?;
        let runtime = self.chat_mount.locked()?;
        fee.validate(pool, VACANCY_READ_METADATA_BYTES)?;
        self.require_no_pending_checkpoint_join_group(context.group)?;
        require_base(self, &runtime)?;
        bounds.require_absent(&runtime)?;
        if runtime.predecessors.excludes_group(context.group) {
            return Err(refused());
        }
        let admission = ColdV2GenesisVacantReadAdmission {
            store: self,
            runtime: &runtime,
            source,
            key: &key,
            bounds: &bounds,
            basis: ColdV2GenesisVacancyBasis::capture(source, &key)?,
            _not_send: PhantomData,
        };
        admission.validate_source_only()?;
        let result = callback(&admission)?;
        admission.validate_source_only()?;
        // Explicit order covers callback errors by reverse local declaration as well.
        drop(runtime);
        drop(bounds);
        drop(key);
        drop(fee);
        Ok(result)
    }
}

/// SAME cached new owner on actual row1, not an absence role. Dirty visibility is negative
/// only; no pin/head or storage operation accepts this type without its durable successor.
pub(in crate::store) struct BornV2GenesisAdmission<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2PreInitializingPreparation,
    _not_send: PhantomData<Rc<()>>,
}
/// Created only AFTER the latest-table file+vault-parent barrier under continuously held
/// AD. Fixed basis is populated from THIS actual held row by the concrete Scope helper
/// before any callback receives this borrower; raw planned-record capture stays private.
pub(in crate::store) struct DurableBornV2GenesisRow1Admission<'writer> {
    original: BornV2GenesisAdmission<'writer>,
    permit: ChatAdmissionStoragePermit<'writer>,
    basis: OnceLock<ColdV2GenesisScopeBasis>,
}

fn check_owner_invoice() -> Result<(), AppError> {
    let named = std::mem::size_of::<ColdV2PreInitializingPreparation>()
        + std::mem::size_of::<ColdV2InitializingPreparation>()
        + std::mem::size_of::<BornScopeOwnerResources>()
        + std::mem::size_of::<BirthProgress>()
        + std::mem::size_of::<AdmissionOwner>()
        + std::mem::size_of::<revocation::Owner>()
        + revocation::LIFE_ALLOCATION_BYTES
        + 2 * std::mem::size_of::<AdmissionAuthority>()
        + 2 * GROUP_BOUNDS_ALLOCATION_BYTES
        + 4 * MAX_GROUP
        + 2 * std::mem::size_of::<OriginalV2GenesisGroupObservation>()
        + 2 * std::mem::size_of::<ColdV2GenesisScopeBasis>()
        + std::mem::size_of::<BornV2GenesisAdmission<'static>>()
        + std::mem::size_of::<DurableBornV2GenesisRow1Admission<'static>>()
        + 16 * 1024; // closed watch/Arc/weak/authority setup, not allocator/RSS admission
    if named > BORN_SCOPE_OWNER_FEE {
        return Err(refused());
    }
    Ok(())
}
impl BornScopeOwnerResources {
    fn prepare(store: &ServerStore) -> Result<Arc<Self>, AppError> {
        check_owner_invoice()?;
        let pool = &store
            .current_layout_policy
            .as_ref()
            .ok_or_else(refused)?
            .archive
            .pending
            .pool;
        let fee = pool.try_reserve(BORN_SCOPE_OWNER_FEE)?;
        Ok(Arc::new(Self { fee }))
    }
    fn validate(&self, store: &ServerStore) -> Result<(), AppError> {
        let pool = &store
            .current_layout_policy
            .as_ref()
            .ok_or_else(refused)?
            .archive
            .pending
            .pool;
        self.fee.validate(pool, BORN_SCOPE_OWNER_FEE)
    }
}
impl ColdV2PreInitializingPreparation {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn owner_data_for_test(&self) -> (Arc<()>, [u8; 32]) {
        (self.inner.owner_identity(), self.inner.nonce())
    }
    pub(in crate::store) fn cancel_original(&self) {
        ManagedChatScopeRevocation::revoke(&self.inner.revoker);
    }
    pub(in crate::store) fn born_repair_seed(&self) -> Option<&Arc<BornV2GenesisRepairSeed>> {
        match &self.progress {
            BirthProgress::Row1Pending => None,
            BirthProgress::RepairSeedSaved(seed) => Some(seed),
        }
    }
    pub(in crate::store) fn require_original_source_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        require_source_pair(store, source)?;
        self.origin.retains_original(store, source)?;
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        self.vacancy.require_same_source(source, &self.inner.key)?;
        let context = self.origin.expectation_data().context.spec_data();
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
            || p.record.phase != Phase::Initializing
            || p.record.revision != 1
            || context.planned_revision != 1
        {
            return Err(refused());
        }
        Ok(())
    }
}
fn require_born_row(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &ColdV2PreInitializingPreparation,
) -> Result<(), AppError> {
    require_base(store, runtime)?;
    preparation.require_original_source_negative(store, source, task)?;
    let p = &preparation.inner;
    preparation.bounds.require_no_foreign(runtime, &p.key)?;
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
    if row != &p.record
        || owner.nonce != row.nonce
        || owner.revision != 1
        || !owner.life.live()
        || !Arc::ptr_eq(&owner.life.identity(), &p.owner_identity)
    {
        return Err(refused());
    }
    Ok(())
}
fn born<'writer>(
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2PreInitializingPreparation,
) -> BornV2GenesisAdmission<'writer> {
    BornV2GenesisAdmission {
        store,
        runtime,
        source,
        task,
        preparation,
        _not_send: PhantomData,
    }
}
impl BornV2GenesisAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.source
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.task
    }
    pub(in crate::store) fn pre_initializing_seed(&self) -> &Arc<ColdV2PreInitializingSeed> {
        &self.preparation.origin
    }
    pub(in crate::store) fn born_repair_seed(&self) -> Option<&Arc<BornV2GenesisRepairSeed>> {
        self.preparation.born_repair_seed()
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
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        require_born_row(
            self.store,
            self.runtime,
            self.source,
            self.task,
            self.preparation,
        )
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
impl DurableBornV2GenesisRow1Admission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.original.store()
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.original.source()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.original.task()
    }
    pub(in crate::store) fn pre_initializing_seed(&self) -> &Arc<ColdV2PreInitializingSeed> {
        self.original.pre_initializing_seed()
    }
    pub(in crate::store) fn owner_identity(&self) -> &Arc<()> {
        self.original.owner_identity()
    }
    pub(in crate::store) fn owner_observer(&self) -> &RequestCancellation {
        self.original.owner_observer()
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.original.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.original.signer()
    }
    pub(in crate::store) fn born_admission(&self) -> &BornV2GenesisAdmission<'_> {
        &self.original
    }
    pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
        self.basis
            .get()
            .expect("private basis initialized after actual durable row1 and before callback")
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.original.validate_source_only()?;
        self.permit
            .validate_original(self.store(), self.original.group(), self.original.signer())
    }
    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<
        crate::store::chat_admission::current_lineage::CurrentFilenameAssociation<'_>,
        AppError,
    > {
        self.validate_source_only()?;
        self.original.current_filename_association()
    }
    /// Only Scope's concrete basis helper can borrow this. The key/Record belong to the
    /// SAME held Runtime AFTER file+parent success, not a planned record from the source.
    pub(super) fn actual_row_for_basis(&self) -> Result<(&ChatAdmissionKey, &Record), AppError> {
        self.validate_source_only()?;
        let p = &self.original.preparation.inner;
        let row = self
            .original
            .runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&p.key)
            .ok_or_else(refused)?;
        if self.original.runtime.dirty || row.phase != Phase::Initializing || row.revision != 1 {
            return Err(refused());
        }
        Ok((&p.key, row))
    }
}

/// Root's physical invoices add their actually retained borrower/basis copies separately
/// from the resource-only64KiB owner class, source/SDK and temporary scope serialization.
pub(in crate::store) const COLD_PREINITIALIZING_SCOPE_METADATA_BYTES: usize = 2
    * std::mem::size_of::<ColdV2GenesisVacancyBasis>()
    + std::mem::size_of::<ColdV2GenesisVacantReadAdmission<'static>>()
    + std::mem::size_of::<BornV2GenesisAdmission<'static>>()
    + std::mem::size_of::<DurableBornV2GenesisRow1Admission<'static>>()
    + std::mem::size_of::<RetiredBornScopeCustody>();

fn validate_born_origin_read(admission: &BornV2GenesisAdmission<'_>) -> Result<(), AppError> {
    admission.validate_source_only()?;
    let guard = admission.task().lock_born_v2_genesis_read(admission)?;
    admission
        .pre_initializing_seed()
        .validate_under_archive_root(admission.store(), admission.source(), guard.archive_root())?;
    guard.validate_source_only()
}
impl ServerStore {
    /// Atomically cache this NEW owner before the first index/row transfer. No attempt to
    /// call a generic fresh/legacy initializer or derive actual ColdBasis from this plan.
    pub(in crate::store) fn prepare_cold_v2_genesis_birth(
        &self,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        origin: Arc<ColdV2PreInitializingSeed>,
        slot: &mut Option<ColdV2PreInitializingPreparation>,
    ) -> Result<(), AppError> {
        origin.retains_original(self, source)?;
        source.validate_original(self)?;
        let resources = if slot.is_none() {
            Some(BornScopeOwnerResources::prepare(self)?)
        } else {
            None
        };
        let context = origin.expectation_data().context.spec_data();
        let key = ChatAdmissionKey::new(context.group, context.signer)?;
        let bounds = if slot.is_none() {
            Some(ChatAdmissionGroupBounds::new(context.group)?)
        } else {
            None
        };
        let candidate = Record {
            nonce: context.scope_nonce,
            revision: 1,
            phase: Phase::Initializing,
            origin: Origin::Fresh,
            provenance: context.fresh_attempt,
        };
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(context.group)?;
        require_base(self, &runtime)?;
        if slot.is_none() {
            let bound_ref = bounds.as_ref().ok_or_else(refused)?;
            let resource = resources.as_ref().ok_or_else(refused)?;
            resource.validate(self)?;
            bound_ref.require_absent(&runtime)?;
            if runtime.predecessors.excludes_group(context.group)
                || runtime.table.as_ref().ok_or_else(refused)?.len()
                    >= MAX_CHAT_ADMISSION_IDENTITIES
            {
                return Err(refused());
            }
            task.require_registered_cold_preinitializing_claim(self, source, &origin)?;
            let basis = ColdV2GenesisVacancyBasis::capture(source, &key)?;
            if &basis != origin.vacancy_basis_data() {
                return Err(refused());
            }
            let admission = ColdV2GenesisVacantReadAdmission {
                store: self,
                runtime: &runtime,
                source,
                key: &key,
                bounds: bound_ref,
                basis: basis.clone(),
                _not_send: PhantomData,
            };
            {
                let guard = task.lock_pre_initializing_birth(&admission)?;
                origin.validate_under_archive_root(self, source, guard.archive_root())?;
                guard.validate_source_only()?;
            }
            let owner_key = key.clone();
            let table_key = key.clone();
            let (inner, owner) =
                self.new_reservation_with_custody(&key, &candidate, resource.clone())?;
            // Repeat physical/absence negatives after watch/key setup. The resource body
            // pays ONLY accounting; authority still comes from original source/claim/AD.
            {
                let guard = task.lock_pre_initializing_birth(&admission)?;
                origin.validate_under_archive_root(self, source, guard.archive_root())?;
                guard.validate_source_only()?;
            }
            if inner.mount.is_cancelled() || inner.admission.is_cancelled() {
                return Err(refused());
            }
            *slot = Some(ColdV2PreInitializingPreparation {
                inner,
                bounds: bounds.expect("prepaid before Runtime"),
                task: task.observe_original(),
                vacancy: basis,
                origin: origin.clone(),
                progress: BirthProgress::Row1Pending,
                owner_resources: resource.clone(),
            });
            // No recoverable work after cache. The authenticated latest map and prebuilt
            // keys are present; ordinary allocator OOM remains abort semantics, not a quota.
            runtime.predecessors.before_row_change(&key, None);
            runtime.owners.insert(owner_key, owner);
            runtime.observe_new_current_lineage_key(&key);
            runtime
                .table
                .as_mut()
                .expect("loaded before actual cache")
                .insert(table_key, candidate);
            runtime.dirty = true;
        }
        let preparation = slot.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(&origin, &preparation.origin) {
            return Err(refused());
        }
        require_born_row(self, &runtime, source, task, preparation)?;
        task.require_born_v2_genesis_scope_owner(&born(self, &runtime, source, task, preparation))
    }

    /// Publish actual row1 durability BEFORE the Root successor callback under the SAME
    /// admission. A dirty retry resyncs the latest same-owned row, never rechecks vacancy.
    pub(in crate::store) fn save_born_v2_genesis_row1(
        &self,
        preparation: &mut ColdV2PreInitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
        capture: impl for<'writer> FnOnce(
            &DurableBornV2GenesisRow1Admission<'writer>,
        ) -> Result<Arc<BornV2GenesisRepairSeed>, AppError>,
    ) -> Result<(), AppError> {
        require_buffers(self, buffers)?;
        preparation.owner_resources.validate(self)?;
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        require_buffers(self, buffers)?;
        require_born_row(self, &runtime, source, task, preparation)?;
        task.require_born_v2_genesis_scope_owner(&born(self, &runtime, source, task, preparation))?;
        validate_born_origin_read(&born(self, &runtime, source, task, preparation))?;
        // Even a successful prior/lost write is no reusable dependent storage permission.
        // The old physical journal is still complete and pin/head remain absent here.
        self.flush_chat_admissions(&mut runtime, rng)?;
        require_buffers(self, buffers)?;
        validate_born_origin_read(&born(self, &runtime, source, task, preparation))?;
        if preparation.born_repair_seed().is_none() {
            let receipt = {
                let durable = DurableBornV2GenesisRow1Admission {
                    original: born(self, &runtime, source, task, preparation),
                    permit: ChatAdmissionStoragePermit::initializing(
                        self,
                        &runtime,
                        &preparation.inner,
                    ),
                    basis: OnceLock::new(),
                };
                // ONLY this concrete post-barrier type can reach the raw private capture.
                let actual = super::cold_read::capture_born_scope_basis(&durable)?;
                durable.basis.set(actual).map_err(|_| refused())?;
                durable.validate_source_only()?;
                capture(&durable)?
            };
            if !Arc::ptr_eq(receipt.pre_initializing_seed(), &preparation.origin) {
                return Err(refused());
            }
            // Cache before any subsequent owner/file observation can refuse or lose reply.
            preparation.progress = BirthProgress::RepairSeedSaved(receipt);
        }
        let admission = born(self, &runtime, source, task, preparation);
        admission
            .born_repair_seed()
            .ok_or_else(refused)?
            .retains_original(self, source, task, admission.owner_identity())?;
        admission.validate_source_only()
    }

    /// A SAME Runtime clean row1 loan for the one-time task handoff. This is neither the
    /// post-write DurableBorn permission nor a generic vacant/Initializing issuer.
    pub(in crate::store) fn with_born_v2_genesis_admission<T>(
        &self,
        preparation: &ColdV2PreInitializingPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        callback: impl for<'writer> FnOnce(&BornV2GenesisAdmission<'writer>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        preparation.owner_resources.validate(self)?;
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        let admission = born(self, &runtime, source, task, preparation);
        admission.validate_source_only()?;
        if runtime.dirty || preparation.born_repair_seed().is_none() {
            return Err(refused());
        }
        validate_born_origin_read(&admission)?;
        let result = callback(&admission)?;
        admission.validate_source_only()?;
        Ok(result)
    }

    /// Move this actual same-row owner into the existing Init1 state machine. All checks,
    /// physical proofs and successor setup precede born.take(); no recoverable work follows
    /// it. Returned old DATA has no cancellation owner and drains OUTSIDE every guard.
    pub(in crate::store) fn convert_born_to_initializing(
        &self,
        born_slot: &mut Option<ColdV2PreInitializingPreparation>,
        target: &mut Option<ColdV2InitializingPreparation>,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<RetiredBornScopeCustody, AppError> {
        if target.is_some() {
            return Err(refused());
        }
        let p = born_slot.as_ref().ok_or_else(refused)?;
        p.owner_resources.validate(self)?;
        let receipt = p.born_repair_seed().ok_or_else(refused)?;
        let seed = receipt.repair_seed().clone();
        let basis = receipt.scope_basis_data().clone();
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(p.inner.group())?;
        let admission = born(self, &runtime, source, task, p);
        admission.validate_source_only()?;
        if runtime.dirty {
            return Err(refused());
        }
        task.require_born_v2_genesis_handoff(&admission)?;
        {
            let guard = task.lock_born_v2_genesis_read(&admission)?;
            receipt.validate_under_archive_root(self, source, guard.archive_root())?;
            guard.validate_source_only()?;
        }
        task.require_born_v2_genesis_handoff(&admission)?;
        // AD remains held through these two-slot field moves. No validation, callback,
        // allocation, refund or cancellation is permitted in this commit interval.
        let previous = born_slot.take().expect("checked exact birth slot under AD");
        let ColdV2PreInitializingPreparation {
            inner,
            bounds,
            task: observation,
            vacancy: _,
            origin,
            progress,
            owner_resources,
        } = previous;
        let BirthProgress::RepairSeedSaved(receipt) = progress else {
            unreachable!("checked cached born receipt")
        };
        *target = Some(ColdV2InitializingPreparation::from_born_parts(
            inner,
            bounds,
            observation,
            basis,
            seed,
            owner_resources,
        ));
        let retired = RetiredBornScopeCustody {
            _origin: origin,
            _receipt: receipt,
        };
        drop(runtime);
        Ok(retired)
    }
}
