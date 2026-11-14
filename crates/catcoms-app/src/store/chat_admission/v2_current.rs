//! Genesis scope admission paired with the actual namespace, saved-artifact receipts and
//! unexposed owning caller. Cold recovery roles are a separate integration boundary below.
//!
//! This is deliberately separate from generic legacy preparation/finish: those paths refuse
//! an archive target, may choose a nonce, and finish without advancing the durable revision.
//! Here an actual paid source and independently saved activation seed fix the sole row effect.
//! No decoded label, public constructor, status flag or test watch can issue this reservation.
use super::group_lineage::{ChatAdmissionGroupBounds, GROUP_BOUNDS_ALLOCATION_BYTES};
use super::*;
use crate::store::critical_namespaces::{
    archive_current_codec::SCOPE_BUFFER_FEE,
    current_layout::{OriginalV2GenesisGroupObservation, PreparedV2GenesisGroupNamespace},
    read_pool::{CatalogReadReservation, CriticalCatalogReadPool},
};
use crate::store::owner_evidence::canonical::v2_registration::V2WriterObservation;
use crate::store::persistence::{
    v2_genesis_activation::{SavedV2GenesisActivationSeed, SavedV2GenesisHead},
    GenesisSourceDataRef,
};
use catcoms_sync::local_chat_scope::{
    ManagedChatScopeBinding, ManagedChatScopeReservation, ManagedChatScopeRevocation,
};
use std::marker::PhantomData;
use std::rc::Rc;

mod association;
mod cold_initializing;
mod cold_preinitializing;
pub(in crate::store) use cold_initializing::{
    ColdInitializingGenesisAdmission, ColdInitializingGenesisHeadAdmission,
    ColdRepairedGenesisCurrentAdmission, ColdRepairedGenesisCurrentProofAdmission,
    ColdV2InitializingPreparation, DurableColdGenesisInitializingAdmission,
    COLD_INITIALIZING_SCOPE_METADATA_BYTES,
};
pub(in crate::store) use cold_preinitializing::{
    BornV2GenesisAdmission, ColdV2GenesisVacancyBasis, ColdV2GenesisVacantReadAdmission,
    ColdV2PreInitializingPreparation, DurableBornV2GenesisRow1Admission, RetiredBornScopeCustody,
    COLD_PREINITIALIZING_SCOPE_METADATA_BYTES,
};
pub(in crate::store) mod cold_owned;
mod cold_read;
mod current_validation;
mod inventory_resync;
mod settlement;
mod text;
pub(in crate::store) use cold_owned::{
    ColdCurrentProofAdmission, ColdOwnedCurrentAdmission, ColdV2CurrentPreparation,
    ColdV2CurrentProgressData, COLD_V2_CURRENT_SCOPE_METADATA_BYTES,
};
pub(in crate::store) use cold_read::ColdV2InitializingReadAdmission;
pub(in crate::store) use cold_read::{ColdV2CurrentReadAdmission, ColdV2GenesisScopeBasis};
pub(in crate::store) use settlement::{
    DurableGenesisCurrent2SettlementAdmission, GenesisCurrent2SettlementReadAdmission,
    SettledGenesisCurrent2ReadAdmission, GENESIS_SEED_SETTLEMENT_SCOPE_METADATA_BYTES,
};
pub(in crate::store) use text::{
    V2TextCurrentAdmission, V2TextScopeNegative, V2_TEXT_SCOPE_METADATA_BYTES,
};
// Cold read and ownership roles are consumed together by the complete seed-present
// Current2 library caller. They do not grant a runnable node or native activation.

/// Original negative scope plus the actual artifact/task identities. Only this implementation
/// can construct it. The actual SavedSeed Arc also retains the candidate metadata payment;
/// its weak task observation alone would not pay for this object's owned metadata.
pub(in crate::store) struct V2CurrentPreparation {
    inner: ChatAdmissionPreparation,
    group_bounds: ChatAdmissionGroupBounds,
    task: OriginalV2GenesisGroupObservation,
    activation: Arc<SavedV2GenesisActivationSeed>,
    progress: Progress,
}

// The inline binding is covered by the actual activation metadata invoice. Boxing this
// phase would add an allocation at the scope/head commit edge, after its fallible preflight.
#[allow(clippy::large_enum_variant)]
enum Progress {
    // The exact row1 candidate is cached before insertion. A failed global write leaves the
    // SAME latest Runtime table dirty; this phase never certifies a successful scope barrier.
    Initializing,
    // Cache the REAL head proof BEFORE either row or AdmissionOwner advances to revision2.
    // The actual table may still be row1 at this point. Retry inspects that exact effect;
    // it cannot rerun the head callback or turn a row2 into an Initializing1 storage permit.
    HeadSavedPendingCurrent2(Arc<SavedV2GenesisHead>),
    CurrentSaved {
        head: Arc<SavedV2GenesisHead>,
        binding: V2CurrentBinding,
    },
}

/// A real saved row2 binding whose historical planned revision stays1. This is private scope
/// custody, not the stronger V2 canonical writer, a runnable node or a governor issuer.
#[derive(Clone)]
pub(in crate::store) struct V2CurrentBinding {
    inner: ChatAdmissionBinding,
    // LAST: this ORIGINAL writer's real8KiB account follows every old cancellation/identity
    // generation, even when its registration drops and another writer later occupies the key.
    writer: V2WriterObservation,
}

/// Named requested metadata inside Root's real retained512KiB activation-candidate invoice.
/// Weak observations do not retain a payment by themselves: the actual SavedSeed Arc in the
/// preparation keeps that original invoice alive. Include owned key copies, the two scope
/// adapters/owner/closed authorities and bounded Arc/watch/Notify setup slack. Waiting futures,
/// existing global BTree metadata, SDK/provider/source and scope serialization are separate.
pub(in crate::store) const V2_CURRENT_SCOPE_METADATA_BYTES: usize =
    std::mem::size_of::<V2CurrentPreparation>()
        + std::mem::size_of::<V2CurrentBinding>()
        + std::mem::size_of::<AdmissionOwner>()
        + std::mem::size_of::<revocation::Owner>()
        + revocation::LIFE_ALLOCATION_BYTES
        + 2 * std::mem::size_of::<AdmissionAuthority>()
        + std::mem::size_of::<OriginalV2GenesisGroupObservation>()
        + 64
        + 2 * GROUP_BOUNDS_ALLOCATION_BYTES
        + 3 * 256
        + 16 * 1024;

macro_rules! reservation {
    ($ty:ty, $inner:ident) => {
        impl ManagedChatScopeReservation for $ty {
            fn group(&self) -> &[u8] {
                self.$inner.group()
            }
            fn signer(&self) -> &[u8; 32] {
                self.$inner.signer()
            }
            fn domain(&self) -> [u8; 32] {
                self.$inner.domain()
            }
            fn nonce(&self) -> [u8; 32] {
                self.$inner.nonce()
            }
            fn planned_revision_data(&self) -> Option<u64> {
                Some(1)
            }
            fn owner_identity(&self) -> Arc<()> {
                self.$inner.owner_identity()
            }
            fn reservation_observer(&self) -> RequestCancellation {
                self.$inner.reservation_observer()
            }
            fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
                // The existing fixed Arc allocation is paid by the preparation's metadata
                // custody. The owning runtime attaches outside host writers, not in a guard.
                self.$inner.revocation()
            }
        }
    };
}
reservation!(V2CurrentPreparation, inner);
impl ManagedChatScopeReservation for V2CurrentBinding {
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
        self.writer.cancellation()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        self.inner.revocation()
    }
}
impl ManagedChatScopeBinding for V2CurrentBinding {
    fn publication_authority(&self) -> RequestCancellation {
        self.writer.cancellation()
    }
}

impl std::fmt::Debug for V2CurrentPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2CurrentPreparation")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2CurrentBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2CurrentBinding").finish_non_exhaustive()
    }
}
impl V2CurrentPreparation {
    pub(in crate::store) fn activation(&self) -> &Arc<SavedV2GenesisActivationSeed> {
        &self.activation
    }
    /// Negative consistency for Root's prepaid writer-life construction OUTSIDE writers.
    /// This neither reads Runtime nor installs a canonical writer; the actual borrowed
    /// Initializing owner guard must separately prove the real row before installation.
    pub(in crate::store) fn require_original_source_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        require_source_pair(store, source)?;
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        self.activation.retains_original(store, source, task)?;
        let context = self.activation.expectation_data().context.spec_data();
        let p = &self.inner;
        if p.mount.is_cancelled()
            || p.admission.is_cancelled()
            || !Arc::ptr_eq(&p.mount_identity, &store.chat_mount.identity)
            || p.group() != context.group
            || p.signer() != &context.signer
            || p.domain != context.scope_domain
            || p.record.nonce != context.scope_nonce
            || p.record.revision != 1
            || p.record.phase != Phase::Initializing
            || p.record.origin != Origin::Fresh
            || p.record.provenance != context.fresh_attempt
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn current_binding(&self) -> Result<&V2CurrentBinding, AppError> {
        match &self.progress {
            Progress::CurrentSaved { binding, .. } => Ok(binding),
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn saved_head(&self) -> Option<&Arc<SavedV2GenesisHead>> {
        match &self.progress {
            Progress::Initializing => None,
            Progress::HeadSavedPendingCurrent2(head) | Progress::CurrentSaved { head, .. } => {
                Some(head)
            }
        }
    }
}

/// Actual source/vacant-row serializer BEFORE any activation seed exists. There is no seed
/// Option or role Boolean: this distinct type cannot be passed to the Head writer/finisher.
pub(in crate::store) struct VacantV2GenesisAdmission<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    key: &'writer ChatAdmissionKey,
    group_bounds: &'writer ChatAdmissionGroupBounds,
    // Like ChatAdmissionStoragePermit, even a borrowed admission cannot be sent to a
    // scoped worker while another thread owns the original synchronous Runtime writer.
    _not_send: PhantomData<Rc<()>>,
}
impl VacantV2GenesisAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.source
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        require_source_pair(self.store, self.source)?;
        require_base(self.store, self.runtime)?;
        self.group_bounds.require_absent(self.runtime)?;
        if self.runtime.dirty || self.runtime.predecessors.excludes_group(self.key.group()) {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn require_vacant_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.validate_source_only()?;
        task.retains_original(self.store, self.source)
    }
}

/// An already-held Runtime borrow for this SAME post-seed initialization. It exposes no raw
/// Runtime, alternate source selector or mutable row. The namespace guard must retain this
/// borrow, so its pins/canonical/Root guards cannot escape the original admission writer.
pub(in crate::store) struct V2CurrentAdmission<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer V2CurrentPreparation,
    _not_send: PhantomData<Rc<()>>,
}
impl V2CurrentAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.source
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.task
    }
    pub(in crate::store) fn activation(&self) -> &Arc<SavedV2GenesisActivationSeed> {
        &self.preparation.activation
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.preparation.inner.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.preparation.inner.signer()
    }
    /// Closed negative observations only: no locks, I/O, SDK, pool operation or mutable
    /// callback. RequestCancellation may invoke the existing audited weak/atomic authorities;
    /// this is not a callback-free contract for arbitrary public trait implementations.
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        require_base(self.store, self.runtime)?;
        self.preparation
            .require_original_source_negative(self.store, self.source, self.task)?;
        require_owned_row(self.store, self.runtime, self.preparation)
    }
    pub(in crate::store) fn require_current_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.preparation.task.require_same_original(task)?;
        self.validate_source_only()?;
        let record = self
            .runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&self.preparation.inner.key)
            .ok_or_else(refused)?;
        if record.phase != Phase::Current
            || record.revision != 2
            || self.preparation.saved_head().is_none()
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// Only the clean actual Initializing1 branch constructs this stronger borrow. It cannot be
/// recreated at Current2, including on a failed Current scope sync. Root's Head writer takes
/// this type through the distinct Initializing group guard, never a vacant/current guard.
pub(in crate::store) struct V2InitializingAdmission<'writer> {
    admission: V2CurrentAdmission<'writer>,
    permit: ChatAdmissionStoragePermit<'writer>,
}
impl<'writer> V2InitializingAdmission<'writer> {
    pub(in crate::store) fn scope_permit(&self) -> &ChatAdmissionStoragePermit<'writer> {
        &self.permit
    }
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.admission.store()
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.admission.source()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.admission.task()
    }
    pub(in crate::store) fn activation(&self) -> &Arc<SavedV2GenesisActivationSeed> {
        self.admission.activation()
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.admission.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.admission.signer()
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_source_only()?;
        self.permit
            .validate_original(self.store(), self.group(), self.signer())
    }
    pub(in crate::store) fn require_initializing_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.admission
            .preparation
            .task
            .require_same_original(task)?;
        self.validate_source_only()
    }
}

fn require_base(store: &ServerStore, runtime: &Runtime) -> Result<(), AppError> {
    if !runtime.managed
        || runtime.format != Some(catcoms_storage::VaultFormat::CriticalNamespaces)
        || runtime.table.is_none()
        || !store.chat_mount.life.live()
    {
        return Err(refused());
    }
    Ok(())
}
fn require_source_pair(
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
) -> Result<(), AppError> {
    source.require_original_negative(store)?;
    source
        .seed()
        .check_opened_source_data(source.source().data())?;
    let data = source.data();
    let context = data.context.spec_data();
    if context.scope_domain != store.chat_vault_domain()? || context.planned_revision != 1 {
        return Err(refused());
    }
    Ok(())
}
fn require_owned_row(
    store: &ServerStore,
    runtime: &Runtime,
    preparation: &V2CurrentPreparation,
) -> Result<(), AppError> {
    // This first supported genesis is a sole-founder role. Any companion/migration needs its
    // own actual source/role; matching our own key cannot hide another signer's group history.
    preparation
        .group_bounds
        .require_no_foreign(runtime, &preparation.inner.key)?;
    let p = &preparation.inner;
    if p.mount.is_cancelled()
        || p.admission.is_cancelled()
        || !Arc::ptr_eq(&p.mount_identity, &store.chat_mount.identity)
        || p.record.revision != 1
        || p.record.phase != Phase::Initializing
        || p.record.origin != Origin::Fresh
    {
        return Err(refused());
    }
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(&p.key)
        .ok_or_else(refused)?;
    let owner = runtime.owners.get(&p.key).ok_or_else(refused)?;
    let exact_phase = (row.phase == Phase::Initializing && row.revision == 1)
        || (row.phase == Phase::Current && row.revision == 2 && preparation.saved_head().is_some());
    if !exact_phase
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
fn pool(store: &ServerStore) -> Result<&CriticalCatalogReadPool, AppError> {
    Ok(&store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool)
}
fn require_buffers(store: &ServerStore, buffers: &CatalogReadReservation) -> Result<(), AppError> {
    buffers.validate(pool(store)?, SCOPE_BUFFER_FEE)
}

/// Existing exact SavedSeed/source/task validation under read-only Root while Runtime is
/// held. It intentionally skips pins/canonical ONLY for resyncing an already authorized row
/// candidate. Nothing escapes this block; release Root before scope I/O or any later writer.
fn validate_saved_seed_read(
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    seed: &SavedV2GenesisActivationSeed,
) -> Result<(), AppError> {
    seed.retains_original(store, source, task)?;
    let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
    seed.validate_under_archive_root(store, source, task, root.as_ref().ok_or_else(refused)?)
}

impl ServerStore {
    /// Resource-only reservation BEFORE any Runtime/pin/Root writer. No source-write unit,
    /// persistent execution grant, Current or authority follows from this temporary credit.
    pub(in crate::store) fn reserve_v2_genesis_scope_buffers(
        &self,
    ) -> Result<CatalogReadReservation, AppError> {
        pool(self)?.try_reserve(SCOPE_BUFFER_FEE)
    }

    /// Factory's pre-seed entry. The private higher-ranked callback cannot return a borrowed
    /// admission/group guard. It may produce only its actual owned negative task/directory or
    /// physical SavedSeed; it never obtains Initializing/Current custody here.
    pub(in crate::store) fn with_vacant_v2_genesis_admission<T>(
        &self,
        source: &GenesisSourceDataRef<'_>,
        callback: impl for<'writer> FnOnce(&VacantV2GenesisAdmission<'writer>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        source.validate_original(self)?;
        // No SavedSeed/candidate payment exists at this pre-seed boundary yet. Pay these
        // two range keys, the local exact key and their fixed wrapper independently BEFORE
        // creating any of them. Declaration order keeps the fee last, outside Runtime Drop.
        let bounds_bytes = GROUP_BOUNDS_ALLOCATION_BYTES
            + 256
            + std::mem::size_of::<ChatAdmissionKey>()
            + std::mem::size_of::<VacantV2GenesisAdmission<'_>>()
            + 64;
        let bounds_fee = pool(self)?.try_reserve(bounds_bytes)?;
        let data = source.data();
        let context = data.context.spec_data();
        let key = ChatAdmissionKey::new(context.group, context.signer)?;
        let group_bounds = ChatAdmissionGroupBounds::new(context.group)?;
        let runtime = self.chat_mount.locked()?;
        bounds_fee.validate(pool(self)?, bounds_bytes)?;
        self.require_no_pending_checkpoint_join_group(key.group())?;
        let admission = VacantV2GenesisAdmission {
            store: self,
            runtime: &runtime,
            source,
            key: &key,
            group_bounds: &group_bounds,
            _not_send: PhantomData,
        };
        admission.validate_source_only()?;
        // This source read can wait on Root after Runtime. Factory's later full guard repeats
        // all source, namespace and original directory proofs immediately before its effect.
        source.validate_original(self)?;
        admission.validate_source_only()?;
        callback(&admission)
    }

    /// Issue ONLY the historical planned row from this actual independently saved activation.
    /// The caller owns `slot` before any effect. Caching precedes table mutation, so uncertain
    /// scope I/O cannot cause another nonce, owner identity, source read or SDK reconstruction.
    pub(in crate::store) fn reserve_v2_genesis_initializing(
        &self,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        activation: Arc<SavedV2GenesisActivationSeed>,
        slot: &mut Option<V2CurrentPreparation>,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        require_buffers(self, scope_buffers)?;
        activation.retains_original(self, source, task)?;
        source.validate_original(self)?;
        let expected = activation.expectation_data();
        let context = expected.context.spec_data();
        let key = ChatAdmissionKey::new(context.group, context.signer)?;
        // These two bounded group-key allocations happen BEFORE taking Runtime. The actual
        // retained metadata invoice, not a source wire limit or Weak observation, pays them.
        let group_bounds = if slot.is_none() {
            Some(ChatAdmissionGroupBounds::new(context.group)?)
        } else {
            None
        };
        let candidate = Record {
            nonce: context.scope_nonce,
            revision: 1,
            phase: Phase::Initializing,
            origin: Origin::Fresh,
            // Historical provenance from the actual selected SourceSaved receipt, NOT a new
            // Fresh witness or permission to found/rejoin from this serialized attempt label.
            provenance: context.fresh_attempt,
        };
        let mut runtime = self.chat_mount.locked()?;
        require_buffers(self, scope_buffers)?;
        require_base(self, &runtime)?;
        self.require_no_pending_checkpoint_join_group(key.group())?;
        if let Some(original) = slot.as_ref() {
            if !Arc::ptr_eq(&original.activation, &activation)
                || original.inner.key != key
                || original.inner.record != candidate
            {
                return Err(refused());
            }
            original.task.require_same_original(task)?;
            require_owned_row(self, &runtime, original)?;
        } else {
            let admission = VacantV2GenesisAdmission {
                store: self,
                runtime: &runtime,
                source,
                key: &key,
                group_bounds: group_bounds.as_ref().ok_or_else(refused)?,
                _not_send: PhantomData,
            };
            admission.require_vacant_lineage_for_own_group_task(task)?;
            if runtime.table.as_ref().ok_or_else(refused)?.len() >= MAX_CHAT_ADMISSION_IDENTITIES {
                return Err(refused());
            }
            // Source/seed/task are actual original objects, not a presumed vacant filesystem.
            // This proof is READ ONLY. Root is released entirely before global scope writing.
            {
                // A live warm SavedDirectory is not group vacancy once a new complete-
                // journal cold task has registered. Hold the actual pending/pins/canonical
                // guard during the seed proof; AD stays held until owner+row transfer.
                let guard = task.lock_vacant(&admission)?;
                activation.validate_under_archive_root(self, source, task, guard.archive_root())?;
                guard.validate_source_only()?;
            }
            admission.validate_source_only()?;
            let owner_key = key.clone();
            let table_key = key.clone();
            // The ACTUAL saved activation's metadata payment follows both Runtime's owner
            // and every observer/revoker tail. This receipt contains no graph/key/sender.
            let (inner, owner) =
                self.new_reservation_with_custody(&key, &candidate, activation.clone())?;
            // Arc/watch/observer setup can outlive the previous proof. Repeat the actual
            // file/source/task and absent-row/owner negatives AFTER those allocations.
            {
                let guard = task.lock_vacant(&admission)?;
                activation.validate_under_archive_root(self, source, task, guard.archive_root())?;
                guard.validate_source_only()?;
            }
            admission.validate_source_only()?;
            if inner.mount.is_cancelled() || inner.admission.is_cancelled() {
                return Err(refused());
            }
            *slot = Some(V2CurrentPreparation {
                inner,
                group_bounds: group_bounds.ok_or_else(refused)?,
                task: task.observe_original(),
                activation,
                progress: Progress::Initializing,
            });
            // No recoverable fallible work/callback between cache and owner transfer. The
            // BTree insertion retains ordinary infallible/OOM-abort behavior; this is not a
            // claim that it allocates no nodes or that the fee reserves global allocator RAM.
            runtime.predecessors.before_row_change(&key, None);
            runtime.owners.insert(owner_key, owner);
            runtime.observe_new_current_lineage_key(&key);
            runtime
                .table
                .as_mut()
                .ok_or_else(refused)?
                .insert(table_key, candidate);
            runtime.dirty = true;
        }
        let preparation = slot.as_ref().ok_or_else(refused)?;
        require_owned_row(self, &runtime, preparation)?;
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&preparation.inner.key)
            .ok_or_else(refused)?;
        if row.phase == Phase::Current {
            // A lost return may already have advanced this SAME owner. Scope1 reservation is
            // not replayed over Current2; the dedicated finisher validates its retained Head.
            return validate_current_head(self, &runtime, source, task, preparation);
        }
        clean_initializing(
            self,
            &mut runtime,
            source,
            task,
            preparation,
            scope_buffers,
            rng,
        )
    }

    /// Cover the actual Head once, then save Current2 under the SAME admission serializer.
    /// Only Root's private physical writer is a production caller of this callback. It receives
    /// actual clean scope1 custody, never a raw field/status-based authorization switch.
    pub(in crate::store) fn finish_v2_genesis_current<R: CryptoRngCore>(
        &self,
        preparation: &mut V2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope_buffers: &CatalogReadReservation,
        rng: &mut R,
        save: impl for<'writer> FnOnce(
            &V2InitializingAdmission<'writer>,
            &mut R,
        ) -> Result<Arc<SavedV2GenesisHead>, AppError>,
    ) -> Result<(), AppError> {
        require_buffers(self, scope_buffers)?;
        preparation
            .activation
            .retains_original(self, source, task)?;
        let mut runtime = self.chat_mount.locked()?;
        require_buffers(self, scope_buffers)?;
        require_base(self, &runtime)?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        require_owned_row(self, &runtime, preparation)?;
        let initializing = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&preparation.inner.key)
            .ok_or_else(refused)?
            .phase
            == Phase::Initializing;
        if initializing {
            clean_initializing(
                self,
                &mut runtime,
                source,
                task,
                preparation,
                scope_buffers,
                rng,
            )?;
            if preparation.saved_head().is_none() {
                let head = {
                    let admission =
                        initializing_admission(self, &runtime, source, task, preparation)?;
                    admission.validate_source_only()?;
                    save(&admission, rng)?
                };
                if !Arc::ptr_eq(head.activation(), &preparation.activation) {
                    return Err(refused());
                }
                head.retains_original(self, source, task)?;
                // This irreversible phase latch is BEFORE row/owner revision mutation. Every
                // later error retains the actual Head and cannot invoke `save` a second time.
                preparation.progress = Progress::HeadSavedPendingCurrent2(head);
            }
            {
                let admission = initializing_admission(self, &runtime, source, task, preparation)?;
                // Canonical writer registration happens in Root's owning caller while this
                // first guard owns pins/canonical but BEFORE it acquires archive Root. On a
                // retained-head retry no registration is created or replaced here.
                let owner_namespace = task.lock_initializing_owner_namespace(&admission)?;
                let group = owner_namespace.lock_archive_root()?;
                preparation
                    .saved_head()
                    .ok_or_else(refused)?
                    .validate_under_archive_root(self, source, task, group.archive_root())?;
                group.validate_source_only()?;
            }
            require_owned_row(self, &runtime, preparation)?;
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
            // Do NOT call reservation_locked between these updates. Its mismatch handling
            // deliberately revokes an owner; this transition must preserve the SAME life.
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

    /// Revalidate/resync ONLY an already issued Current2 after the graph moves into its Node.
    /// The source facade still borrows the SAME read/SDK/resource owners. There is deliberately
    /// no Head callback argument here: pairing cannot enable replay of scope1/Head effects.
    pub(in crate::store) fn resync_v2_genesis_current(
        &self,
        preparation: &mut V2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if !matches!(&preparation.progress, Progress::CurrentSaved { .. }) {
            return Err(refused());
        }
        require_buffers(self, scope_buffers)?;
        preparation
            .activation
            .retains_original(self, source, task)?;
        let mut runtime = self.chat_mount.locked()?;
        require_buffers(self, scope_buffers)?;
        require_base(self, &runtime)?;
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
}

fn finish_current2(
    store: &ServerStore,
    runtime: &mut Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &mut V2CurrentPreparation,
    scope_buffers: &CatalogReadReservation,
    rng: &mut impl CryptoRngCore,
) -> Result<(), AppError> {
    validate_current_head(store, runtime, source, task, preparation)?;
    if runtime.dirty {
        require_buffers(store, scope_buffers)?;
        // Resource validation may wait; full original source/Head proofs follow it. There is
        // no quota acquisition or arbitrary callback before the synchronous scope writer.
        validate_current_head(store, runtime, source, task, preparation)?;
        store.flush_chat_admissions(runtime, rng)?;
    }
    // A successful32MiB write proves no source/task/writer liveness. Recheck all actual
    // originals AFTER it, before constructing or returning any Current scope binding.
    validate_current_head(store, runtime, source, task, preparation)?;
    if !matches!(&preparation.progress, Progress::CurrentSaved { .. }) {
        let binding = store.bind_owned_chat_admission_locked(
            runtime,
            &preparation.inner.key,
            preparation.inner.nonce(),
            2,
            &preparation.inner.owner_identity,
        )?;
        require_owned_row(store, runtime, preparation)?;
        let head = preparation.saved_head().ok_or_else(refused)?.clone();
        let writer = head.writer_observation().clone();
        if writer.is_cancelled() {
            return Err(refused());
        }
        preparation.progress = Progress::CurrentSaved {
            head,
            binding: V2CurrentBinding {
                inner: binding,
                writer,
            },
        };
    }
    // Sticky actual binding; a retry never samples another owner or watch generation.
    if preparation
        .current_binding()?
        .reservation_observer()
        .is_cancelled()
    {
        return Err(refused());
    }
    V2CurrentAdmission {
        store,
        runtime,
        source,
        task,
        preparation,
        _not_send: PhantomData,
    }
    .validate_source_only()
}

fn clean_initializing(
    store: &ServerStore,
    runtime: &mut Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &V2CurrentPreparation,
    scope_buffers: &CatalogReadReservation,
    rng: &mut impl CryptoRngCore,
) -> Result<(), AppError> {
    require_owned_row(store, runtime, preparation)?;
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(&preparation.inner.key)
        .ok_or_else(refused)?;
    if row.phase != Phase::Initializing || row.revision != 1 {
        return Err(refused());
    }
    require_buffers(store, scope_buffers)?;
    validate_saved_seed_read(store, source, task, &preparation.activation)?;
    preparation.task.require_same_original(task)?;
    task.retains_original(store, source)?;
    require_owned_row(store, runtime, preparation)?;
    if runtime.dirty {
        // Serialize the LATEST whole table under its one real writer. Never restore a cached
        // table image or fabricate per-row durable floors; unrelated group rows stay intact.
        store.flush_chat_admissions(runtime, rng)?;
    }
    validate_saved_seed_read(store, source, task, &preparation.activation)?;
    preparation.task.require_same_original(task)?;
    task.retains_original(store, source)?;
    require_owned_row(store, runtime, preparation)
}

fn initializing_admission<'writer>(
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer V2CurrentPreparation,
) -> Result<V2InitializingAdmission<'writer>, AppError> {
    let permit = ChatAdmissionStoragePermit::initializing(store, runtime, &preparation.inner);
    let result = V2InitializingAdmission {
        admission: V2CurrentAdmission {
            store,
            runtime,
            source,
            task,
            preparation,
            _not_send: PhantomData,
        },
        permit,
    };
    result.validate_source_only()?;
    Ok(result)
}

fn validate_current_head(
    store: &ServerStore,
    runtime: &Runtime,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &V2CurrentPreparation,
) -> Result<(), AppError> {
    let admission = V2CurrentAdmission {
        store,
        runtime,
        source,
        task,
        preparation,
        _not_send: PhantomData,
    };
    admission.require_current_lineage_for_own_group_task(task)?;
    let group = task.lock_current(&admission)?;
    let head = preparation.saved_head().ok_or_else(refused)?;
    if !Arc::ptr_eq(head.activation(), &preparation.activation) {
        return Err(refused());
    }
    head.validate_under_archive_root(store, source, task, group.archive_root())?;
    group.validate_source_only()
}

#[cfg(test)]
mod tests;
