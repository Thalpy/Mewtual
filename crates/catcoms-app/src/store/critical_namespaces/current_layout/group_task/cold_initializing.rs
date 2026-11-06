//! Cold Initializing1 claims. No warm directory state is manufactured here. Actual
//! Scope owns the sole sender; this task retains only a funded original negative association.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdInitializingGenesisAdmission, ColdInitializingGenesisHeadAdmission,
    ColdRepairedGenesisCurrentAdmission, ColdRepairedGenesisCurrentProofAdmission,
    ColdV2GenesisScopeBasis, DurableColdGenesisInitializingAdmission,
};
use crate::store::owner_evidence::canonical::current_vacancy::cold_initializing as owners;
use crate::store::owner_evidence::canonical::v2_registration::{
    cold_initializing::PreparedColdV2InitializingWriterRegistration, V2WriterObservation,
};
use crate::store::persistence::v2_genesis_activation::initializing_repair::ColdV2GenesisRepairSeed;
use crate::store::persistence::v2_genesis_activation::ColdV2GenesisReadResources;
use std::sync::OnceLock;

pub(super) struct ColdInitializingClaim {
    seed: Arc<ColdV2GenesisRepairSeed>,
    resources: ColdV2GenesisReadResources,
    // Only the successful actual registry guard may issue this opaque original witness.
    registered: Option<owners::RegisteredColdInitializingClaim>,
    owner: OnceLock<ColdInitializingOwnerAssociation>,
}
struct ColdInitializingOwnerAssociation {
    identity: Arc<()>,
    basis: ColdV2GenesisScopeBasis,
    // LAST: actual watch and authority tails keep the SAME real cold metadata payment.
    negative: RequestCancellation,
}

/// Fixed prepared negative association. Only canonical's concrete Root-proved handoff can
/// take a slot; this setter alone owns no registry access or Current permission.
pub(in crate::store) struct PreparedColdInitializingOwnerAssociation<'task, 'writer> {
    task: &'task PreparedV2GenesisGroupNamespace,
    admission: &'task ColdInitializingGenesisAdmission<'writer>,
    association: Option<ColdInitializingOwnerAssociation>,
}
impl PreparedColdInitializingOwnerAssociation<'_, '_> {
    pub(in crate::store) fn install(
        self,
        task: &PreparedV2GenesisGroupNamespace,
        admission: &ColdInitializingGenesisAdmission<'_>,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.task, task) || !std::ptr::eq(self.admission, admission) {
            return Err(refused());
        }
        task.require_cold_initializing_scope_owner(admission)?;
        if let Some(association) = self.association {
            if association.negative.is_cancelled() {
                return Err(refused());
            }
            // Err leaves the existing cell unchanged. Success is followed only by the
            // canonical caller's private infallible exact slot move.
            task.cold_initializing()?
                .owner
                .set(association)
                .map_err(|_| refused())
        } else {
            task.require_cold_initializing_owned_handoff(admission)
        }
    }
}

// Separate named task metadata from Root's cold file/receipt loan. Native fixed selectors
// are actually exercised by the owned/proof guards and retain their independent allowance.
const INITIALIZING_TASK_FEE: usize = GROUP_TASK_STAMP_FEE
    + std::mem::size_of::<PreparedColdInitializingOwnerAssociation<'static, 'static>>()
    + 2 * std::mem::size_of::<ColdInitializingClaim>()
    + 2 * std::mem::size_of::<ColdInitializingOwnerAssociation>()
    + 2 * std::mem::size_of::<owners::RegisteredColdInitializingClaim>()
    + std::mem::size_of::<owners::ColdInitializingV2GenesisOwnerNamespace<'static, 'static>>()
    + std::mem::size_of::<owners::ColdRepairedV2GenesisCurrentProofNamespace<'static, 'static>>()
    + 4 * (super::super::MAX_OPEN_BASE_PATH + 256)
    + 32 * 1024
    + 2 * std::mem::size_of::<String>()
    + 64
    + 128;

impl PreparedV2GenesisGroupNamespace {
    fn cold_initializing(&self) -> Result<&ColdInitializingClaim, AppError> {
        match &self.custody {
            GroupTaskCustody::ColdInitializing(cold) => Ok(cold),
            GroupTaskCustody::Warm { .. } | GroupTaskCustody::ColdCurrent(_) => Err(refused()),
        }
    }
    pub(in crate::store) fn cold_initializing_seed_data(
        &self,
    ) -> Option<&Arc<ColdV2GenesisRepairSeed>> {
        match &self.custody {
            GroupTaskCustody::ColdInitializing(cold) => Some(&cold.seed),
            GroupTaskCustody::Warm { .. } | GroupTaskCustody::ColdCurrent(_) => None,
        }
    }
    pub(in crate::store) fn cold_initializing_resources(
        &self,
    ) -> Result<&ColdV2GenesisReadResources, AppError> {
        Ok(&self.cold_initializing()?.resources)
    }
    pub(in crate::store) fn require_registered_cold_initializing_claim(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: &Arc<ColdV2GenesisRepairSeed>,
    ) -> Result<(), AppError> {
        self.retains_original(store, source)?;
        let cold = self.cold_initializing()?;
        if cold.owner.get().is_some() || !Arc::ptr_eq(&cold.seed, seed) {
            return Err(refused());
        }
        cold.resources.require_original_negative(store, source)?;
        cold.registered
            .as_ref()
            .ok_or_else(refused)?
            .require_original(self, seed)
    }
    // Keep the complete identity tuple explicit at this shared comparison boundary. Each
    // typed entry point supplies actual source/head/basis/owner/observer loans; no field is
    // optional, and omitting one would let an equal label substitute for original custody.
    #[allow(clippy::too_many_arguments)]
    fn require_initializing_owner_data(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: &Arc<ColdV2GenesisRepairSeed>,
        basis: &ColdV2GenesisScopeBasis,
        identity: &Arc<()>,
        observer: &RequestCancellation,
        require_handoff: bool,
    ) -> Result<(), AppError> {
        self.retains_original(store, source)?;
        let cold = self.cold_initializing()?;
        cold.resources.require_original_negative(store, source)?;
        if !Arc::ptr_eq(&cold.seed, seed)
            || seed.scope_basis_data() != basis
            || observer.is_cancelled()
        {
            return Err(refused());
        }
        match cold.owner.get() {
            Some(owner)
                if Arc::ptr_eq(&owner.identity, identity)
                    && &owner.basis == basis
                    && !owner.negative.is_cancelled() =>
            {
                Ok(())
            }
            Some(_) => Err(refused()),
            None if require_handoff => Err(refused()),
            None => self.require_registered_cold_initializing_claim(store, source, seed),
        }
    }
    /// Negative only: actual role owns phase/clean gates. Never call its lineage wrapper
    /// here, which itself conjoins this matcher. The source-only comparator is acyclic.
    pub(in crate::store) fn require_cold_initializing_scope_owner(
        &self,
        admission: &ColdInitializingGenesisAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        self.require_initializing_owner_data(
            admission.store(),
            admission.source(),
            admission.seed(),
            admission.scope_basis_data(),
            admission.owner_identity(),
            admission.owner_observer(),
            false,
        )
    }
    pub(in crate::store) fn require_cold_initializing_owned_handoff(
        &self,
        admission: &ColdInitializingGenesisAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        self.require_initializing_owner_data(
            admission.store(),
            admission.source(),
            admission.seed(),
            admission.scope_basis_data(),
            admission.owner_identity(),
            admission.owner_observer(),
            true,
        )
    }
    /// Claim the actual complete activation seed on unowned Initializing1 under a NEW
    /// paid read/SDK and physical mount. Prefix progress is retained by Root's receipt,
    /// never by a synthetic warm SavedDirectory or complete Current2 receipt.
    /// Everything prepared before writers; after actual slot installation only moves remain.
    pub(in crate::store) fn from_cold_initializing(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: Arc<ColdV2GenesisRepairSeed>,
    ) -> Result<Self, AppError> {
        source.require_original_negative(store)?;
        seed.retains_original(store, source)?;
        let policy = store.current_layout_policy.as_ref().ok_or_else(refused)?;
        let fee = Arc::new(
            policy
                .archive
                .pending
                .pool
                .try_reserve(INITIALIZING_TASK_FEE)?,
        );
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
            custody: GroupTaskCustody::ColdInitializing(ColdInitializingClaim {
                seed,
                resources,
                registered: None,
                owner: OnceLock::new(),
            }),
            retired: None,
            life: Arc::new(GroupTaskLife {
                cancelled: AtomicBool::new(false),
            }),
            fee,
        };
        let mut prepared =
            owners::prepare_registration(store, &policy.archive.pending.pool, &task)?;
        let registered = store.with_cold_v2_genesis_initializing_read(source, |admission| {
            let mut owner = store.lock_registering_cold_initializing_namespace(admission, &task)?;
            let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            owner.register_under_archive_root(&mut prepared, root.as_ref().ok_or_else(refused)?)
        })?;
        // No callback/check/allocation in the commit-to-witness move. Unused prepared
        // backing drains/refunds here OUTSIDE guards before return; a concurrent close
        // merely makes the SAME returned DATA task negative, never a renewed claim.
        if let GroupTaskCustody::ColdInitializing(cold) = &mut task.custody {
            cold.registered = Some(registered);
        }
        Ok(task)
    }

    /// Actual owner handoff, not directory-only retirement. Caller MUST drain returned paid
    /// debt outside its admission callback. Task/owner negatives remain original afterward.
    pub(in crate::store) fn handoff_cold_initializing_owner(
        &self,
        admission: &ColdInitializingGenesisAdmission<'_>,
    ) -> Result<
        Option<crate::store::owner_evidence::canonical::current_vacancy::RetiredGroupEntry>,
        AppError,
    > {
        let store = admission.store();
        let cold = self.cold_initializing()?;
        // These clones do not allocate. All fixed storage and escaping tails are actually
        // prepaid; observe/compare the original owner AFTER clone/setup and full waits.
        let association = if cold.owner.get().is_none() {
            Some(ColdInitializingOwnerAssociation {
                identity: admission.owner_identity().clone(),
                basis: admission.scope_basis_data().clone(),
                negative: admission.owner_observer().clone(),
            })
        } else {
            None
        };
        let prepared = PreparedColdInitializingOwnerAssociation {
            task: self,
            admission,
            association,
        };
        let mut owner = store.lock_cold_initializing_owned_namespace(admission, self)?;
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        owner.handoff_under_archive_root(prepared, root.as_ref().ok_or_else(refused)?)
    }
}

/// Named simultaneous Factory layouts/selectors added to the NEW Root repair invoice.
/// Root files/prefix frames/native two-folder inventory are separate terms; task/registry
/// and writer generation/backing each retain their own actual payment through their tails.
pub(in crate::store) const COLD_INITIALIZING_GROUP_GUARD_BYTES: usize =
    std::mem::size_of::<ColdInitializingV2GenesisOwnerGuard<'static, 'static>>()
        + std::mem::size_of::<ColdInitializingV2GenesisRepairGroupGuard<'static, 'static>>()
        + std::mem::size_of::<ColdInitializingV2GenesisReadGroupGuard<'static, 'static>>()
        + std::mem::size_of::<ColdRepairedV2GenesisCurrentProofGroupGuard<'static, 'static>>()
        + std::mem::size_of::<ColdRepairedV2GenesisCurrentGroupGuard<'static, 'static>>()
        + 4 * (super::super::MAX_OPEN_BASE_PATH + 256)
        + 32 * 1024;

pub(in crate::store) struct ColdInitializingV2GenesisOwnerGuard<'task, 'writer> {
    owner: owners::DurableColdInitializingV2GenesisOwnerNamespace<'task, 'writer>,
}
pub(in crate::store) struct ColdInitializingV2GenesisRepairGroupGuard<'task, 'writer> {
    root: MutexGuard<'task, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::DurableColdInitializingV2GenesisOwnerNamespace<'task, 'writer>,
    registration: &'task PreparedColdV2InitializingWriterRegistration,
}
pub(in crate::store) struct ColdInitializingV2GenesisReadGroupGuard<'task, 'writer> {
    root: MutexGuard<'task, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdInitializingV2GenesisReadNamespace<'task, 'writer>,
}
pub(in crate::store) struct ColdRepairedV2GenesisCurrentProofGroupGuard<'task, 'writer> {
    root: MutexGuard<'task, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdRepairedV2GenesisCurrentProofNamespace<'task, 'writer>,
}
pub(in crate::store) struct ColdRepairedV2GenesisCurrentGroupGuard<'task, 'writer> {
    root: MutexGuard<'task, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdRepairedV2GenesisCurrentNamespace<'task, 'writer>,
}
macro_rules! root_guard {
    ($ty:ident,$role:ident) => {
        impl $ty<'_, '_> {
            pub(in crate::store) fn scope_admission(&self) -> &$role<'_> {
                self.owner.admission()
            }
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.scope_admission().store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.scope_admission().source()
            }
            pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
                self.scope_admission().task()
            }
            pub(in crate::store) fn seed(&self) -> &Arc<ColdV2GenesisRepairSeed> {
                self.scope_admission().seed()
            }
            pub(in crate::store) fn archive_root(
                &self,
            ) -> &super::super::super::archive_bootstrap::MountedRoot {
                self.root
                    .as_ref()
                    .expect("actual mounted Root checked before concrete guard")
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.owner.validate_source_only()
            }
            fn validate_physical(&self) -> Result<(), AppError> {
                self.owner.validate_old_files()?;
                self.seed().validate_under_archive_root(
                    self.store(),
                    self.source(),
                    self.archive_root(),
                )?;
                self.store()
                    .validate_mounted_current_layout_under_archive_root(self.archive_root())?;
                let (path, file, id) = self.seed().directory_data();
                crate::store::persistence::v2_genesis_activation::require_directory_name(
                    path, file, id,
                )?;
                self.validate_source_only()
            }
        }
    };
}
root_guard!(
    ColdInitializingV2GenesisRepairGroupGuard,
    DurableColdGenesisInitializingAdmission
);
root_guard!(
    ColdInitializingV2GenesisReadGroupGuard,
    ColdInitializingGenesisHeadAdmission
);
root_guard!(
    ColdRepairedV2GenesisCurrentProofGroupGuard,
    ColdRepairedGenesisCurrentProofAdmission
);
root_guard!(
    ColdRepairedV2GenesisCurrentGroupGuard,
    ColdRepairedGenesisCurrentAdmission
);

impl<'task, 'writer> ColdInitializingV2GenesisOwnerGuard<'task, 'writer> {
    pub(in crate::store) fn install_initializing_writer(
        &mut self,
        writer: &mut PreparedColdV2InitializingWriterRegistration,
    ) -> Result<(), AppError> {
        self.owner.install_initializing_writer(writer)
    }
    pub(in crate::store) fn lock_archive_root_for_initializing_writer(
        mut self,
        registration: &'task PreparedColdV2InitializingWriterRegistration,
    ) -> Result<ColdInitializingV2GenesisRepairGroupGuard<'task, 'writer>, AppError> {
        self.owner
            .require_installed_initializing_writer(registration)?;
        // SAME installed owner/registration remains borrowed until all Root I/O drains.
        let root = self
            .owner
            .admission()
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let guard = ColdInitializingV2GenesisRepairGroupGuard {
            root,
            owner: self.owner,
            registration,
        };
        guard.validate_physical()?;
        Ok(guard)
    }
}
impl ColdInitializingV2GenesisRepairGroupGuard<'_, '_> {
    pub(in crate::store) fn writer_registration(
        &self,
    ) -> &PreparedColdV2InitializingWriterRegistration {
        self.registration
    }
    pub(in crate::store) fn resources(&self) -> &ColdV2GenesisReadResources {
        // Constructor already checked the concrete custody family; no account acquisition.
        &self
            .task()
            .cold_initializing()
            .expect("private Initializing guard")
            .resources
    }
}
macro_rules! existing_writer_proof {
    ($ty:ident) => {
        impl $ty<'_, '_> {
            pub(in crate::store) fn require_installed_initializing_observation(
                &self,
                observation: &V2WriterObservation,
            ) -> Result<(), AppError> {
                self.owner
                    .require_installed_initializing_observation(observation)
            }
        }
    };
}
existing_writer_proof!(ColdInitializingV2GenesisReadGroupGuard);
existing_writer_proof!(ColdRepairedV2GenesisCurrentProofGroupGuard);
existing_writer_proof!(ColdRepairedV2GenesisCurrentGroupGuard);

impl PreparedV2GenesisGroupNamespace {
    pub(in crate::store) fn lock_cold_initializing_owner_namespace<'task, 'writer>(
        &'task self,
        admission: &'task DurableColdGenesisInitializingAdmission<'writer>,
    ) -> Result<ColdInitializingV2GenesisOwnerGuard<'task, 'writer>, AppError> {
        Ok(ColdInitializingV2GenesisOwnerGuard {
            owner: admission
                .store()
                .lock_durable_cold_initializing_namespace(admission, self)?,
        })
    }
}
macro_rules! lock_read {
    ($method:ident,$role:ident,$guard:ident,$lock:ident) => {
        impl PreparedV2GenesisGroupNamespace {
            pub(in crate::store) fn $method<'task, 'writer>(
                &'task self,
                admission: &'task $role<'writer>,
            ) -> Result<$guard<'task, 'writer>, AppError> {
                let owner = admission.store().$lock(admission, self)?;
                let root = admission
                    .store()
                    .archive_bootstrap_root
                    .lock()
                    .map_err(|_| refused())?;
                root.as_ref().ok_or_else(refused)?;
                let guard = $guard { root, owner };
                guard.validate_physical()?;
                guard.require_installed_initializing_observation(
                    admission.head().writer_observation(),
                )?;
                Ok(guard)
            }
        }
    };
}
lock_read!(
    lock_cold_initializing_head_read,
    ColdInitializingGenesisHeadAdmission,
    ColdInitializingV2GenesisReadGroupGuard,
    lock_cold_initializing_head_namespace
);
lock_read!(
    lock_cold_repaired_current_proof,
    ColdRepairedGenesisCurrentProofAdmission,
    ColdRepairedV2GenesisCurrentProofGroupGuard,
    lock_cold_repaired_current_proof_namespace
);
lock_read!(
    lock_cold_repaired_current,
    ColdRepairedGenesisCurrentAdmission,
    ColdRepairedV2GenesisCurrentGroupGuard,
    lock_cold_repaired_current_namespace
);

#[cfg(test)]
mod tests;
