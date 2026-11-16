//! Original complete journal/no-row task. Its successor keeps the SAME task Life and
//! original mounted read/SDK; a private exclusive normalization later drains old DATA only.
use super::*;
use crate::store::chat_admission::v2_current::{
    BornV2GenesisAdmission, ColdV2GenesisVacancyBasis, ColdV2GenesisVacantReadAdmission,
    DurableBornV2GenesisRow1Admission,
};
use crate::store::owner_evidence::canonical::current_vacancy::cold_preinitializing as owners;
use crate::store::persistence::v2_genesis_activation::{
    pre_initializing::ColdV2PreInitializingSeed, ColdV2GenesisReadResources,
};
use std::sync::OnceLock;

pub(super) struct ColdPreInitializingClaim {
    origin: Arc<ColdV2PreInitializingSeed>,
    resources: ColdV2GenesisReadResources,
    registered: Option<owners::RegisteredPreInitializingClaim>,
    successor: OnceLock<super::cold_initializing::ColdInitializingClaim>,
}
#[must_use = "drain old pre-initializing metadata outside every host/SDK writer"]
pub(in crate::store) struct RetiredPreInitializingTaskCustody {
    _old: ColdPreInitializingClaim,
}

pub(in crate::store) struct PreparedBornGroupSuccessor<'task, 'writer> {
    task: &'task PreparedV2GenesisGroupNamespace,
    admission: &'task BornV2GenesisAdmission<'writer>,
    successor: Option<super::cold_initializing::ColdInitializingClaim>,
}
impl PreparedBornGroupSuccessor<'_, '_> {
    pub(in crate::store) fn install(
        self,
        task: &PreparedV2GenesisGroupNamespace,
        admission: &BornV2GenesisAdmission<'_>,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.task, task) || !std::ptr::eq(self.admission, admission) {
            return Err(refused());
        }
        task.require_born_v2_genesis_scope_owner(admission)?;
        if let Some(successor) = self.successor {
            // No allocation/drop gap: the canonical caller follows this set ONLY by its
            // private exact slot take. A failed set never changes the original slot.
            task.pre_initializing()?
                .successor
                .set(successor)
                .map_err(|_| refused())
        } else {
            task.require_born_v2_genesis_handoff(admission)
        }
    }
}

const PRE_TASK_FEE: usize = GROUP_TASK_STAMP_FEE
    + 2 * std::mem::size_of::<ColdPreInitializingClaim>()
    + 2 * std::mem::size_of::<super::cold_initializing::ColdInitializingClaim>()
    + 2 * std::mem::size_of::<owners::RegisteredPreInitializingClaim>()
    + std::mem::size_of::<PreparedBornGroupSuccessor<'static, 'static>>()
    + 4 * (super::super::MAX_OPEN_BASE_PATH + 256)
    + 48 * 1024;

impl PreparedV2GenesisGroupNamespace {
    fn pre_initializing(&self) -> Result<&ColdPreInitializingClaim, AppError> {
        match &self.custody {
            GroupTaskCustody::ColdPreInitializing(pre) => Ok(pre),
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn pre_initializing_seed_data(
        &self,
    ) -> Option<&Arc<ColdV2PreInitializingSeed>> {
        match &self.custody {
            GroupTaskCustody::ColdPreInitializing(pre) => Some(&pre.origin),
            _ => None,
        }
    }
    pub(super) fn initializing_successor(
        &self,
    ) -> Option<&super::cold_initializing::ColdInitializingClaim> {
        match &self.custody {
            GroupTaskCustody::ColdPreInitializing(pre) => pre.successor.get(),
            _ => None,
        }
    }
    pub(in crate::store) fn require_registered_cold_preinitializing_claim(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: &Arc<ColdV2PreInitializingSeed>,
    ) -> Result<(), AppError> {
        self.retains_original(store, source)?;
        let pre = self.pre_initializing()?;
        if pre.successor.get().is_some() || !Arc::ptr_eq(&pre.origin, seed) {
            return Err(refused());
        }
        pre.resources.require_original_negative(store, source)?;
        pre.registered
            .as_ref()
            .ok_or_else(refused)?
            .require_original(self, seed)
    }
    pub(in crate::store) fn require_born_v2_genesis_scope_owner(
        &self,
        admission: &BornV2GenesisAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        self.retains_original(admission.store(), admission.source())?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        let pre = self.pre_initializing()?;
        if !Arc::ptr_eq(&pre.origin, admission.pre_initializing_seed()) {
            return Err(refused());
        }
        match pre.successor.get() {
            Some(_) => self.require_born_v2_genesis_handoff(admission),
            None => self.require_registered_cold_preinitializing_claim(
                admission.store(),
                admission.source(),
                admission.pre_initializing_seed(),
            ),
        }
    }
    pub(in crate::store) fn require_born_v2_genesis_handoff(
        &self,
        admission: &BornV2GenesisAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        let receipt = admission.born_repair_seed().ok_or_else(refused)?;
        receipt.retains_original(
            admission.store(),
            admission.source(),
            self,
            admission.owner_identity(),
        )?;
        self.require_initializing_owner_data(
            admission.store(),
            admission.source(),
            receipt.repair_seed(),
            receipt.scope_basis_data(),
            admission.owner_identity(),
            admission.owner_observer(),
            true,
        )
    }
    /// NEW original registered preclaim; no mkdir, scope row, SDK load or fresh nonce.
    pub(in crate::store) fn from_cold_pre_initializing(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: Arc<ColdV2PreInitializingSeed>,
    ) -> Result<Self, AppError> {
        source.require_original_negative(store)?;
        seed.retains_original(store, source)?;
        let policy = store.current_layout_policy.as_ref().ok_or_else(refused)?;
        let fee = Arc::new(policy.archive.pending.pool.try_reserve(PRE_TASK_FEE)?);
        let layout = store
            .current_layout
            .lock()
            .map_err(|_| refused())?
            .as_ref()
            .ok_or_else(refused)?
            .layout;
        let data = source.data();
        if layout.vault != data.context.spec_data().vault {
            return Err(refused());
        }
        let resources = seed.read_resources();
        resources.validate_original(store, source)?;
        let mut task = Self {
            source: OriginalGroupSource {
                id: data.source,
                context: data.context.clone(),
                signer: data.context.spec_data().signer,
                read: source.source().observe_original(),
                sdk: source.seed().observe_original_seed_data(),
                mount: store.registry_mount(),
                negative: store.chat_mount.observer(),
            },
            layout,
            custody: GroupTaskCustody::ColdPreInitializing(ColdPreInitializingClaim {
                origin: seed,
                resources,
                registered: None,
                successor: OnceLock::new(),
            }),
            retired: None,
            life: Arc::new(GroupTaskLife {
                cancelled: AtomicBool::new(false),
            }),
            fee,
        };
        let mut prepared = super::owners::PreparedGroupRegistration::new(
            store,
            &policy.archive.pending.pool,
            &task,
        )?;
        let registered = store.with_cold_v2_genesis_vacant_read(source, |admission| {
            let mut owner = store.lock_registering_pre_initializing_namespace(admission, &task)?;
            let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            owner.register_under_archive_root(
                &task,
                &mut prepared,
                root.as_ref().ok_or_else(refused)?,
            )
        })?;
        // Actual slot committed; witness move is infallible. Prepared unused backing drains
        // outside all guards before return, and a later cancellation cannot renew this task.
        if let GroupTaskCustody::ColdPreInitializing(pre) = &mut task.custody {
            pre.registered = Some(registered);
        }
        Ok(task)
    }
    pub(in crate::store) fn handoff_born_v2_genesis_owner(
        &self,
        admission: &BornV2GenesisAdmission<'_>,
    ) -> Result<Option<super::owners::RetiredGroupEntry>, AppError> {
        self.require_born_v2_genesis_scope_owner(admission)?;
        let pre = self.pre_initializing()?;
        let receipt = admission.born_repair_seed().ok_or_else(refused)?;
        let successor = if pre.successor.get().is_none() {
            Some(super::cold_initializing::ColdInitializingClaim::from_born(
                receipt.repair_seed().clone(),
                admission.owner_identity().clone(),
                receipt.scope_basis_data().clone(),
                admission.owner_observer().clone(),
            ))
        } else {
            None
        };
        let prepared = PreparedBornGroupSuccessor {
            task: self,
            admission,
            successor,
        };
        let mut owner = admission
            .store()
            .lock_born_v2_genesis_namespace(admission)?;
        let root = admission
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?;
        owner.handoff_under_archive_root(prepared, root.as_ref().ok_or_else(refused)?)
    }
    /// Exclusive OUTSIDE writers. Validate shape before take; take→replace consists solely
    /// of infallible field moves. Moving old custody cannot cancel this SAME task Life.
    pub(in crate::store) fn normalize_born_successor(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        scope: &crate::store::chat_admission::v2_current::ColdV2InitializingPreparation,
    ) -> Result<Option<RetiredPreInitializingTaskCustody>, AppError> {
        use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
        // A completed task handoff alone is not the two-slot Scope conversion. Require
        // its actual new same-owner preparation before draining old PreInit custody.
        scope.require_original_source_negative(store, source, self)?;
        self.require_initializing_owner_data(
            store,
            source,
            scope.seed(),
            scope.seed().scope_basis_data(),
            &scope.owner_identity(),
            &scope.reservation_observer(),
            true,
        )?;
        match &self.custody {
            GroupTaskCustody::ColdInitializing(_) => return Ok(None),
            GroupTaskCustody::ColdPreInitializing(pre) if pre.successor.get().is_some() => {}
            _ => return Err(refused()),
        }
        let GroupTaskCustody::ColdPreInitializing(pre) = &mut self.custody else {
            unreachable!("checked kind")
        };
        let successor = pre.successor.take().expect("exclusive checked successor");
        let old = std::mem::replace(
            &mut self.custody,
            GroupTaskCustody::ColdInitializing(successor),
        );
        let GroupTaskCustody::ColdPreInitializing(old) = old else {
            unreachable!("exclusive original kind")
        };
        Ok(Some(RetiredPreInitializingTaskCustody { _old: old }))
    }
}

/// Factory metadata is added to Root's independent loans; old selectors/association are
/// retained while physical capture clones original controls, not overlap-credited away.
pub(in crate::store) const COLD_PREINITIALIZING_GROUP_GUARD_BYTES: usize =
    4 * std::mem::size_of::<ColdV2GenesisVacantGroupReadGuard<'static, 'static>>()
        + 2 * std::mem::size_of::<DurableBornV2GenesisRow1GroupGuard<'static, 'static>>()
        + 4 * (super::super::MAX_OPEN_BASE_PATH + 256)
        + 32 * 1024;

pub(in crate::store) struct ColdV2GenesisVacantGroupReadGuard<'guard, 'writer> {
    directory: crate::store::persistence::OpenedColdV2GroupDirectory<'guard>,
    root: MutexGuard<'guard, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdV2VacantOwnerReadNamespace<'guard, 'writer>,
    resources: &'guard ColdV2GenesisReadResources,
}
impl ColdV2GenesisVacantGroupReadGuard<'_, '_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.owner.admission().store()
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.owner.admission().source()
    }
    pub(in crate::store) fn archive_root(
        &self,
    ) -> &super::super::super::archive_bootstrap::MountedRoot {
        self.root.as_ref().expect("checked Root")
    }
    pub(in crate::store) fn resources(&self) -> &ColdV2GenesisReadResources {
        self.resources
    }
    pub(in crate::store) fn vacancy_basis_data(&self) -> &ColdV2GenesisVacancyBasis {
        self.owner.admission().vacancy_basis_data()
    }
    pub(in crate::store) fn directory_data(
        &self,
    ) -> (
        &std::path::Path,
        &std::fs::File,
        catcoms_storage::kept::SessionId,
    ) {
        self.directory.directory_data()
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.owner.validate_source_only()?;
        self.resources
            .require_original_negative(self.store(), self.source())
    }
    fn validate_physical(&self) -> Result<(), AppError> {
        self.resources
            .validate_original(self.store(), self.source())?;
        self.owner.validate_old_files()?;
        self.source()
            .source()
            .validate_under_archive_root(self.store(), self.archive_root())?;
        self.store()
            .validate_mounted_current_layout_under_archive_root(self.archive_root())?;
        self.directory.require_original_directory()?;
        self.validate_source_only()
    }
}
impl ServerStore {
    pub(in crate::store) fn lock_cold_vacant_genesis_group<'guard, 'writer>(
        &'guard self,
        admission: &'guard ColdV2GenesisVacantReadAdmission<'writer>,
        resources: &'guard ColdV2GenesisReadResources,
    ) -> Result<ColdV2GenesisVacantGroupReadGuard<'guard, 'writer>, AppError> {
        resources.validate_original(self, admission.source())?;
        let owner = self.lock_cold_vacant_owner_namespace(admission)?;
        let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let directory = self.open_cold_v2_group_directory(
            admission.source(),
            root.as_ref().ok_or_else(refused)?,
            resources,
        )?;
        let guard = ColdV2GenesisVacantGroupReadGuard {
            directory,
            root,
            owner,
            resources,
        };
        guard.validate_physical()?;
        Ok(guard)
    }
}

pub(in crate::store) struct BornV2GenesisGroupReadGuard<'guard, 'writer> {
    root: MutexGuard<'guard, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::BornV2GenesisOwnerNamespace<'guard, 'writer>,
}
pub(in crate::store) struct PreInitializingBirthGroupGuard<'guard, 'writer> {
    root: MutexGuard<'guard, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdV2VacantOwnerReadNamespace<'guard, 'writer>,
}
impl PreInitializingBirthGroupGuard<'_, '_> {
    pub(in crate::store) fn archive_root(
        &self,
    ) -> &super::super::super::archive_bootstrap::MountedRoot {
        self.root.as_ref().expect("checked Root")
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.owner.validate_source_only()
    }
}
impl BornV2GenesisGroupReadGuard<'_, '_> {
    pub(in crate::store) fn archive_root(
        &self,
    ) -> &super::super::super::archive_bootstrap::MountedRoot {
        self.root.as_ref().expect("checked Root")
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.owner.validate_source_only()
    }
}
impl PreparedV2GenesisGroupNamespace {
    pub(in crate::store) fn lock_pre_initializing_birth<'guard, 'writer>(
        &'guard self,
        a: &'guard ColdV2GenesisVacantReadAdmission<'writer>,
    ) -> Result<PreInitializingBirthGroupGuard<'guard, 'writer>, AppError> {
        let owner = a.store().lock_pre_initializing_birth_namespace(a, self)?;
        let root = a
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        owner.validate_old_files()?;
        Ok(PreInitializingBirthGroupGuard { root, owner })
    }
    pub(in crate::store) fn lock_born_v2_genesis_read<'guard, 'writer>(
        &'guard self,
        a: &'guard BornV2GenesisAdmission<'writer>,
    ) -> Result<BornV2GenesisGroupReadGuard<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, a.task()) {
            return Err(refused());
        }
        let owner = a.store().lock_born_v2_genesis_namespace(a)?;
        let root = a
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        owner.validate_old_files()?;
        Ok(BornV2GenesisGroupReadGuard { root, owner })
    }
}

pub(in crate::store) struct DurableBornV2GenesisRow1GroupGuard<'guard, 'writer> {
    root: MutexGuard<'guard, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::DurableBornV2GenesisOwnerNamespace<'guard, 'writer>,
    resources: &'guard ColdV2GenesisReadResources,
}
impl DurableBornV2GenesisRow1GroupGuard<'_, '_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.owner.admission().store()
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.owner.admission().source()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.owner.admission().task()
    }
    pub(in crate::store) fn archive_root(
        &self,
    ) -> &super::super::super::archive_bootstrap::MountedRoot {
        self.root.as_ref().expect("checked Root")
    }
    pub(in crate::store) fn resources(&self) -> &ColdV2GenesisReadResources {
        self.resources
    }
    pub(in crate::store) fn pre_initializing_seed(&self) -> &Arc<ColdV2PreInitializingSeed> {
        self.owner.admission().pre_initializing_seed()
    }
    pub(in crate::store) fn owner_identity(&self) -> &Arc<()> {
        self.owner.admission().owner_identity()
    }
    pub(in crate::store) fn owner_observer(&self) -> &RequestCancellation {
        self.owner.admission().owner_observer()
    }
    pub(in crate::store) fn scope_basis_data(
        &self,
    ) -> &crate::store::chat_admission::v2_current::ColdV2GenesisScopeBasis {
        self.owner.admission().scope_basis_data()
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.owner.validate_source_only()?;
        self.resources
            .require_original_negative(self.store(), self.source())
    }
}
impl ServerStore {
    pub(in crate::store) fn lock_durable_born_v2_genesis_group<'guard, 'writer>(
        &'guard self,
        a: &'guard DurableBornV2GenesisRow1Admission<'writer>,
        resources: &'guard ColdV2GenesisReadResources,
    ) -> Result<DurableBornV2GenesisRow1GroupGuard<'guard, 'writer>, AppError> {
        resources.validate_original(self, a.source())?;
        let owner = self.lock_durable_born_v2_genesis_namespace(a)?;
        let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let mounted = root.as_ref().ok_or_else(refused)?;
        owner.validate_old_files()?;
        a.pre_initializing_seed()
            .validate_under_archive_root(self, a.source(), mounted)?;
        let guard = DurableBornV2GenesisRow1GroupGuard {
            root,
            owner,
            resources,
        };
        guard.validate_source_only()?;
        Ok(guard)
    }
}
