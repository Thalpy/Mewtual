//! Original negative group task, paired with the concrete namespace guards, fixed directory
//! writer and actual source/scope consumers. This does not enable operational Chat V2.
//! No directory label, saved-looking bytes or expired task can construct this owner.
use super::*;
use crate::store::chat_admission::v2_current::{
    V2CurrentAdmission, V2InitializingAdmission, VacantV2GenesisAdmission,
};
use crate::store::owner_evidence::canonical::{
    current_vacancy as owners, v2_registration::PreparedV2WriterRegistration,
};
use crate::store::persistence::{GenesisSourceDataRef, OriginalBootstrapReadObservation};
use catcoms_sync::owner_v2::{
    BootstrapSourceContextData, BootstrapSourceId, OriginalV2GenesisSeedObservation,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::MutexGuard;
use std::sync::{Arc, Weak};

pub(in crate::store) mod cold_current;
pub(in crate::store) mod cold_initializing;
pub(in crate::store) mod cold_preinitializing;

/// Warm directory work and cold complete-Current custody have different origins. In
/// particular a cold task never owns a synthetic DirectoryProgress/SavedDirectory.
// The distinct task invoices cover their inline custody. In particular ColdCurrent's
// preallocated OnceLock must not require an allocation at the canonical handoff edge.
#[allow(clippy::large_enum_variant)]
enum GroupTaskCustody {
    Warm {
        output: Arc<crate::store::persistence::V2GenesisDirectoryCustody>,
        directory: Option<Arc<SavedV2GroupDirectory>>,
    },
    ColdCurrent(cold_current::ColdCurrentClaim),
    // Complete original activation journal with a new cold Initializing1 capture. Never
    // synthesize warm directory output or complete-Current2 provenance for this family.
    ColdInitializing(cold_initializing::ColdInitializingClaim),
    ColdPreInitializing(cold_preinitializing::ColdPreInitializingClaim),
}

/// Tiny original lifetime only. Its weak allocations can outlive the task, so every actual
/// observation carries the SAME task metadata payment; no weak pointer pins a source graph.
pub(in crate::store) struct GroupTaskLife {
    cancelled: AtomicBool,
}

/// Full source equality remains data, while the two weak identities prevent adoption of a
/// byte-equal new reader/SDK graph. The actual original source and resource holders stay in
/// the caller's GenesisSourceDataRef, never in inert Store directory debt.
struct OriginalGroupSource {
    id: BootstrapSourceId,
    context: BootstrapSourceContextData,
    signer: [u8; 32],
    read: OriginalBootstrapReadObservation,
    sdk: OriginalV2GenesisSeedObservation,
    mount: Arc<()>,
    negative: RequestCancellation,
}
impl OriginalGroupSource {
    fn require_same(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        source.require_original_negative(store)?;
        self.read.require_same_original(source.source())?;
        self.sdk.require_same_seed_data(source.seed())?;
        let data = source.data();
        if self.negative.is_cancelled()
            || !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || self.id != data.source
            || &self.context != data.context
            || self.signer != data.context.spec_data().signer
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// Negative exact-original observation. There is no sender, source/SDK graph, key, Store or
/// session here. LAST fee ownership covers the original tiny weak control block even after
/// Runtime retires its slot and the task/directory payloads have otherwise drained.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2GenesisGroupObservation {
    life: Weak<GroupTaskLife>,
    _fee: Arc<CatalogReadReservation>,
}
impl OriginalV2GenesisGroupObservation {
    pub(in crate::store) fn require_same_original(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        let original = self.life.upgrade().ok_or_else(refused)?;
        if !Arc::ptr_eq(&original, &task.life)
            || original.cancelled.load(Ordering::Acquire)
            || task.source.negative.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl std::fmt::Debug for OriginalV2GenesisGroupObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalV2GenesisGroupObservation")
            .finish_non_exhaustive()
    }
}

/// One original negative directory owner. The inert output cell is shared with Runtime before
/// any mkdir; it owns no task life and retains partial/saved DATA after caller Drop. Saved
/// directory receipts carry no back-edge to this task or output cell.
pub(in crate::store) struct PreparedV2GenesisGroupNamespace {
    source: OriginalGroupSource,
    layout: super::super::archive_current_namespace_codec::CurrentLayoutData,
    custody: GroupTaskCustody,
    // Retired slot resources must remain outside guarded code until the real permanent fence
    // has taken over. They never contain a task sender or support a replacement registration.
    retired: Option<crate::store::owner_evidence::canonical::current_vacancy::RetiredGroupEntry>,
    life: Arc<GroupTaskLife>,
    // LAST: every source/context/path/output/observation body drains before its account lease.
    fee: Arc<CatalogReadReservation>,
}
impl Drop for PreparedV2GenesisGroupNamespace {
    fn drop(&mut self) {
        self.cancel_original();
        // In particular do not erase Runtime's negative slot here. An admitted directory
        // syscall/result may still need its inert output custody and original resource fee.
    }
}
impl std::fmt::Debug for PreparedV2GenesisGroupNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2GenesisGroupNamespace")
            .finish_non_exhaustive()
    }
}
impl PreparedV2GenesisGroupNamespace {
    /// Negative only and nonblocking. The owning kernel calls this BEFORE node/transport/RNG
    /// payload cleanup; no slot, saved receipt, quota, sender or newer task is adopted/renewed.
    pub(in crate::store) fn cancel_original(&self) {
        self.life.cancelled.store(true, Ordering::Release);
    }
    /// Synchronous outer entry: never invoke while holding an admission/owner/Root guard.
    /// Every I/O refusal retains the SAME output/slot/File and immutable source. No RNG or
    /// reseal is used, and a saved directory never becomes another scope/Current admission.
    pub(in crate::store) fn retry_directory(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        // Cold complete-file custody has no warm mkdir/receipt state to retry.
        self.directory_output()?;
        // Retired fee owners can be last; drain only here, before acquiring any host writer.
        drop(self.retired.take());
        self.retains_original(store, source)?;
        if self.saved_directory().is_err() {
            store.with_vacant_v2_genesis_admission(source, |admission| {
                let guard = self.lock_creating(admission)?;
                store.write_v2_genesis_group_directory(&guard)
            })?;
            // This can fail (e.g. File duplication) without losing the Store-retained saved
            // output. No global writer is held during its path/Arc allocation or cleanup.
            let directory = self.directory_output()?.collect_saved_directory(self)?;
            self.set_warm_saved_directory(directory)?;
        }
        let retired = store.with_vacant_v2_genesis_admission(source, |admission| {
            let mut guard = self.lock_vacant(admission)?;
            guard.validate_source_only()?;
            let directory = guard.directory;
            // Borrow the already-held Root directly. This cannot reconstruct/reenter an
            // admission or infer a physical barrier from the file merely being visible.
            let root = guard.root.as_ref().ok_or_else(refused)?;
            guard.owner.retire_saved_directory(directory, root)
        })?;
        self.retired = retired;
        // Successful transfer releases scarce unresolved metadata, not task/source life.
        // Directory and SAME live task remain owned by the eventual activation consumer.
        drop(self.retired.take());
        Ok(())
    }

    pub(in crate::store) fn observe_original(&self) -> OriginalV2GenesisGroupObservation {
        OriginalV2GenesisGroupObservation {
            life: Arc::downgrade(&self.life),
            _fee: self.fee.clone(),
        }
    }
    /// Cached original comparison only. Physical file/root validation belongs to the distinct
    /// actual role guard; no path check, pool operation or positive authority occurs here.
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        if self.life.cancelled.load(Ordering::Acquire) {
            return Err(refused());
        }
        self.source.require_same(store, source)
    }
    pub(in crate::store) fn saved_directory(
        &self,
    ) -> Result<&Arc<SavedV2GroupDirectory>, AppError> {
        match &self.custody {
            GroupTaskCustody::Warm { directory, .. } => directory.as_ref().ok_or_else(refused),
            GroupTaskCustody::ColdCurrent(_)
            | GroupTaskCustody::ColdInitializing(_)
            | GroupTaskCustody::ColdPreInitializing(_) => Err(refused()),
        }
    }
    fn set_warm_saved_directory(
        &mut self,
        saved: Arc<SavedV2GroupDirectory>,
    ) -> Result<(), AppError> {
        match &mut self.custody {
            GroupTaskCustody::Warm { directory, .. } => {
                *directory = Some(saved);
                Ok(())
            }
            GroupTaskCustody::ColdCurrent(_)
            | GroupTaskCustody::ColdInitializing(_)
            | GroupTaskCustody::ColdPreInitializing(_) => Err(refused()),
        }
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.source.context.spec_data().group
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        &self.source.signer
    }
    pub(in crate::store) fn layout_data(
        &self,
    ) -> super::super::archive_current_namespace_codec::CurrentLayoutData {
        self.layout
    }
    pub(in crate::store) fn source_id_data(&self) -> BootstrapSourceId {
        self.source.id
    }
    pub(in crate::store) fn original_life(&self) -> Weak<GroupTaskLife> {
        Arc::downgrade(&self.life)
    }
    pub(in crate::store) fn directory_output(
        &self,
    ) -> Result<&Arc<crate::store::persistence::V2GenesisDirectoryCustody>, AppError> {
        match &self.custody {
            GroupTaskCustody::Warm { output, .. } => Ok(output),
            GroupTaskCustody::ColdCurrent(_)
            | GroupTaskCustody::ColdInitializing(_)
            | GroupTaskCustody::ColdPreInitializing(_) => Err(refused()),
        }
    }
}

#[cfg(test)]
mod tests;

// Fixed directory I/O and its immutable real barrier receipt live in Store's persistence
// child. Re-exporting that concrete type here preserves the joint Root/Scope interface while
// preventing sibling modules from constructing a saved-looking receipt from raw labels.
pub(in crate::store) use crate::store::persistence::SavedV2GroupDirectory;

/// Named requested buffers/metadata only; existing SourceData, SDK graph, global scope map
/// and native/runtime allocations keep their separate real payments. Context owns one <=256B
/// group copy. Tiny original read/SDK/life controls and actual Arc headers are explicit.
const GROUP_TASK_STAMP_FEE: usize = std::mem::size_of::<PreparedV2GenesisGroupNamespace>()
    + std::mem::size_of::<OriginalGroupSource>()
    + std::mem::size_of::<GroupTaskLife>()
    + std::mem::size_of::<OriginalV2GenesisGroupObservation>()
    + std::mem::size_of::<CatalogReadReservation>()
    + 256
    + std::mem::size_of::<OriginalBootstrapReadObservation>()
    + 64
    + std::mem::size_of::<OriginalV2GenesisSeedObservation>()
    + 64
    + 3 * (2 * std::mem::size_of::<usize>()); // task life, shared fee and source-owning body
const GROUP_TASK_FEE: usize =
    crate::store::persistence::V2_GROUP_DIRECTORY_METADATA_BYTES + GROUP_TASK_STAMP_FEE;

impl V2CurrentLayoutStore {
    pub(in crate::store) fn prepare_genesis_group_namespace(
        &self,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<PreparedV2GenesisGroupNamespace, AppError> {
        let store = self.checked_store()?;
        source.require_original_negative(store)?;
        let policy = store.current_layout_policy.as_ref().ok_or_else(refused)?;
        let fee = Arc::new(policy.archive.pending.pool.try_reserve(GROUP_TASK_FEE)?);
        let data = source.data();
        source
            .seed()
            .check_opened_source_data(source.source().data())?;
        let layout = store
            .current_layout
            .lock()
            .map_err(|_| refused())?
            .as_ref()
            .ok_or_else(refused)?
            .layout;
        if layout.vault != data.context.spec_data().vault {
            return Err(refused());
        }
        let original = OriginalGroupSource {
            id: data.source,
            context: data.context.clone(),
            signer: data.context.spec_data().signer,
            read: source.source().observe_original(),
            sdk: source.seed().observe_original_seed_data(),
            mount: store.registry_mount(),
            negative: store.chat_mount.observer(),
        };
        let output = store.prepare_v2_genesis_directory_output(source, layout, fee.clone())?;
        let task = PreparedV2GenesisGroupNamespace {
            source: original,
            layout,
            custody: GroupTaskCustody::Warm {
                output,
                directory: None,
            },
            retired: None,
            life: Arc::new(GroupTaskLife {
                cancelled: AtomicBool::new(false),
            }),
            fee,
        };
        let mut prepared =
            owners::PreparedGroupRegistration::new(store, &policy.archive.pending.pool, &task)?;
        store.with_vacant_v2_genesis_admission(source, |admission| {
            let mut namespace = store.lock_registering_v2_group_namespace(admission, &task)?;
            let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            namespace.install_under_archive_root(&mut prepared, root)
        })?;
        // Source/task+prepared ownership was complete BEFORE registration. The callback's
        // final entry transfer cannot fail afterward; unused backing drops only OUTSIDE it.
        drop(prepared);
        Ok(task)
    }
}

/// PRIVATE first-directory role. It has no SavedDirectory and cannot be passed to Root's
/// activation seed/head functions. All directory effects remain in the fixed Store child.
pub(in crate::store) struct CreatingV2GenesisDirectoryGuard<'guard, 'writer> {
    owner: owners::CreatingV2OwnerNamespace<'guard, 'writer>,
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
}
pub(in crate::store) struct VacantV2GenesisGroupGuard<'guard, 'writer> {
    owner: owners::VacantV2OwnerNamespace<'guard, 'writer>,
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    directory: &'guard Arc<SavedV2GroupDirectory>,
}
pub(in crate::store) struct InitializingV2GenesisOwnerGuard<'guard, 'writer> {
    owner: owners::InitializingV2OwnerNamespace<'guard, 'writer>,
    directory: &'guard Arc<SavedV2GroupDirectory>,
}
pub(in crate::store) struct InitializingV2GenesisGroupGuard<'guard, 'writer> {
    owner: owners::InitializingV2OwnerNamespace<'guard, 'writer>,
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    directory: &'guard Arc<SavedV2GroupDirectory>,
    registration: &'guard PreparedV2WriterRegistration,
}
/// Init1 proof-only role for an already-retained SavedHead. It cannot enter Root.save_head,
/// has no writer installation capability and does not replay the original Head callback.
pub(in crate::store) struct InitializingV2GenesisReadGuard<'guard, 'writer> {
    owner: owners::InitializingV2OwnerNamespace<'guard, 'writer>,
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    directory: &'guard Arc<SavedV2GroupDirectory>,
}
pub(in crate::store) struct CurrentV2GenesisGroupGuard<'guard, 'writer> {
    owner: owners::CurrentV2OwnerNamespace<'guard, 'writer>,
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    directory: &'guard Arc<SavedV2GroupDirectory>,
}

macro_rules! root_guard_accessors {
    ($name:ident) => {
        impl $name<'_, '_> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.owner.store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.owner.source()
            }
            pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
                self.owner.task()
            }
            pub(in crate::store) fn archive_root(
                &self,
            ) -> &super::super::archive_bootstrap::MountedRoot {
                // The private constructor checks Some before moving this exclusive immutable
                // guard into self. No accessor exposes mutation or can remove the Root.
                self.root
                    .as_ref()
                    .expect("Root checked by private namespace guard constructor")
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.owner.validate_source_only()?;
                let root = self.archive_root();
                let layout = self.task().layout_data();
                if root.negative.is_cancelled()
                    || !Arc::ptr_eq(&root.mount, &self.store().registry_mount())
                    || root.descriptor.vault != layout.vault
                    || root.descriptor.root != layout.root
                {
                    return Err(refused());
                }
                Ok(())
            }
        }
    };
}
root_guard_accessors!(CreatingV2GenesisDirectoryGuard);
root_guard_accessors!(VacantV2GenesisGroupGuard);
root_guard_accessors!(InitializingV2GenesisGroupGuard);
root_guard_accessors!(InitializingV2GenesisReadGuard);
root_guard_accessors!(CurrentV2GenesisGroupGuard);
macro_rules! directory_accessor {
    ($name:ident) => {
        impl $name<'_, '_> {
            pub(in crate::store) fn directory(&self) -> &Arc<SavedV2GroupDirectory> {
                self.directory
            }
        }
    };
}
directory_accessor!(VacantV2GenesisGroupGuard);
directory_accessor!(InitializingV2GenesisGroupGuard);

impl InitializingV2GenesisGroupGuard<'_, '_> {
    pub(in crate::store) fn activation(
        &self,
    ) -> &Arc<crate::store::persistence::v2_genesis_activation::SavedV2GenesisActivationSeed> {
        self.owner.admission().activation()
    }
    pub(in crate::store) fn writer_registration(&self) -> &PreparedV2WriterRegistration {
        self.registration
    }
}
impl<'guard, 'writer> InitializingV2GenesisOwnerGuard<'guard, 'writer> {
    pub(in crate::store) fn install_writer(
        &mut self,
        prepared: &mut PreparedV2WriterRegistration,
    ) -> Result<(), AppError> {
        self.owner.validate_source_only()?;
        self.owner.install_writer(prepared)
    }
    pub(in crate::store) fn lock_archive_root_for_writer(
        mut self,
        registration: &'guard PreparedV2WriterRegistration,
    ) -> Result<InitializingV2GenesisGroupGuard<'guard, 'writer>, AppError> {
        self.owner.require_installed_writer(registration)?;
        let store = self.owner.admission().store();
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let result = InitializingV2GenesisGroupGuard {
            owner: self.owner,
            root,
            directory: self.directory,
            registration,
        };
        result.owner.validate_old_files()?;
        result.directory.validate_under_archive_root(
            result.store(),
            result.source(),
            result.task(),
            result.archive_root(),
        )?;
        result.validate_source_only()?;
        Ok(result)
    }

    /// Read/proof closure for an already retained genuine SavedHead. Its validator must
    /// check the original writer's negative observation; this guard cannot write a Head.
    pub(in crate::store) fn lock_archive_root(
        self,
    ) -> Result<InitializingV2GenesisReadGuard<'guard, 'writer>, AppError> {
        let store = self.owner.admission().store();
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let result = InitializingV2GenesisReadGuard {
            owner: self.owner,
            root,
            directory: self.directory,
        };
        result.owner.validate_old_files()?;
        result.directory.validate_under_archive_root(
            result.store(),
            result.source(),
            result.task(),
            result.archive_root(),
        )?;
        result.validate_source_only()?;
        Ok(result)
    }
}
impl PreparedV2GenesisGroupNamespace {
    fn lock_creating<'guard, 'writer>(
        &'guard self,
        admission: &'guard VacantV2GenesisAdmission<'writer>,
    ) -> Result<CreatingV2GenesisDirectoryGuard<'guard, 'writer>, AppError> {
        let store = admission.store();
        let owner = store.lock_creating_v2_group_namespace(admission, self)?;
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let result = CreatingV2GenesisDirectoryGuard { owner, root };
        result.owner.validate_old_files()?;
        store.validate_mounted_current_layout_under_archive_root(result.archive_root())?;
        admission
            .source()
            .source()
            .validate_under_archive_root(store, result.archive_root())?;
        result.validate_source_only()?;
        Ok(result)
    }
    pub(in crate::store) fn lock_vacant<'guard, 'writer>(
        &'guard self,
        admission: &'guard VacantV2GenesisAdmission<'writer>,
    ) -> Result<VacantV2GenesisGroupGuard<'guard, 'writer>, AppError> {
        let store = admission.store();
        let directory = self.saved_directory()?;
        let owner = store.lock_vacant_v2_group_namespace(admission, self)?;
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let result = VacantV2GenesisGroupGuard {
            owner,
            root,
            directory,
        };
        result.owner.validate_old_files()?;
        result.directory.validate_under_archive_root(
            store,
            result.source(),
            self,
            result.archive_root(),
        )?;
        result.validate_source_only()?;
        Ok(result)
    }
    pub(in crate::store) fn lock_initializing_owner_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard V2InitializingAdmission<'writer>,
    ) -> Result<InitializingV2GenesisOwnerGuard<'guard, 'writer>, AppError> {
        let directory = self.saved_directory()?;
        let owner = admission
            .store()
            .lock_initializing_v2_group_namespace(admission, self)?;
        Ok(InitializingV2GenesisOwnerGuard { owner, directory })
    }
    pub(in crate::store) fn lock_current<'guard, 'writer>(
        &'guard self,
        admission: &'guard V2CurrentAdmission<'writer>,
    ) -> Result<CurrentV2GenesisGroupGuard<'guard, 'writer>, AppError> {
        let store = admission.store();
        let directory = self.saved_directory()?;
        let owner = store.lock_current_v2_group_namespace(admission, self)?;
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let result = CurrentV2GenesisGroupGuard {
            owner,
            root,
            directory,
        };
        result.owner.validate_old_files()?;
        result.directory.validate_under_archive_root(
            store,
            result.source(),
            self,
            result.archive_root(),
        )?;
        result.validate_source_only()?;
        Ok(result)
    }
}
