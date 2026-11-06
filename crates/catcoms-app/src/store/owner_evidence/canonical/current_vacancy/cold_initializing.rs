//! Distinct cold Init1 claim/owner/repair/read namespaces, all on the SAME pending16 slots.
//! Only concrete Scope roles borrow the admission serializer. No namespace creates a row,
//! reconstructs warm Saved provenance or returns a raw canonical map/storage permission.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdInitializingGenesisAdmission, ColdInitializingGenesisHeadAdmission,
    ColdRepairedGenesisCurrentAdmission, ColdRepairedGenesisCurrentProofAdmission,
    ColdV2GenesisScopeBasis, ColdV2InitializingReadAdmission,
    DurableColdGenesisInitializingAdmission,
};
use crate::store::critical_namespaces::current_layout::OriginalV2GenesisGroupObservation;
use crate::store::persistence::v2_genesis_activation::initializing_repair::ColdV2GenesisRepairSeed;

pub(in crate::store) struct RegisteredColdInitializingClaim {
    seed: Arc<ColdV2GenesisRepairSeed>,
    basis: ColdV2GenesisScopeBasis,
    original: OriginalV2GenesisGroupObservation,
}
impl RegisteredColdInitializingClaim {
    pub(super) fn prepare(task: &PreparedV2GenesisGroupNamespace) -> Result<Self, AppError> {
        let seed = task
            .cold_initializing_seed_data()
            .ok_or_else(refused)?
            .clone();
        Ok(Self {
            basis: seed.scope_basis_data().clone(),
            seed,
            original: task.observe_original(),
        })
    }
    pub(in crate::store) fn require_original(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
        seed: &Arc<ColdV2GenesisRepairSeed>,
    ) -> Result<(), AppError> {
        self.original.require_same_original(task)?;
        if !Arc::ptr_eq(&self.seed, seed) || &self.basis != seed.scope_basis_data() {
            return Err(refused());
        }
        Ok(())
    }
}
pub(in crate::store) fn prepare_registration(
    store: &ServerStore,
    pool: &CriticalCatalogReadPool,
    task: &PreparedV2GenesisGroupNamespace,
) -> Result<PreparedGroupRegistration, AppError> {
    task.cold_initializing_seed_data().ok_or_else(refused)?;
    PreparedGroupRegistration::new(store, pool, task)
}

// A closed role sum shares NEGATIVE probes only. Its constructors remain distinct; only
// Durable has mutable writer-map parts, while retained Head/row2 proofs expose read parts.
enum Admission<'task, 'writer> {
    Register(&'task ColdV2InitializingReadAdmission<'writer>),
    Owned(&'task ColdInitializingGenesisAdmission<'writer>),
    Durable(&'task DurableColdGenesisInitializingAdmission<'writer>),
    Head(&'task ColdInitializingGenesisHeadAdmission<'writer>),
    Proof(&'task ColdRepairedGenesisCurrentProofAdmission<'writer>),
    Current(&'task ColdRepairedGenesisCurrentAdmission<'writer>),
}
macro_rules! admission_getter {
    ($self:ident, $method:ident) => {
        match $self {
            Admission::Register(a) => a.$method(),
            Admission::Owned(a) => a.$method(),
            Admission::Durable(a) => a.$method(),
            Admission::Head(a) => a.$method(),
            Admission::Proof(a) => a.$method(),
            Admission::Current(a) => a.$method(),
        }
    };
}
impl<'task, 'writer> Admission<'task, 'writer> {
    fn store(&self) -> &'task ServerStore {
        admission_getter!(self, store)
    }
    fn source(&self) -> &GenesisSourceDataRef<'_> {
        admission_getter!(self, source)
    }
    fn association(&self) -> Result<CurrentFilenameAssociation<'task>, AppError> {
        // Explicit loans keep the association physically tied to this SAME admission.
        match self {
            Self::Register(a) => {
                let a: &'task ColdV2InitializingReadAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Owned(a) => {
                let a: &'task ColdInitializingGenesisAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Durable(a) => {
                let a: &'task DurableColdGenesisInitializingAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Head(a) => {
                let a: &'task ColdInitializingGenesisHeadAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Proof(a) => {
                let a: &'task ColdRepairedGenesisCurrentProofAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Current(a) => {
                let a: &'task ColdRepairedGenesisCurrentAdmission<'writer> = a;
                a.current_filename_association()
            }
        }
    }
    fn validate_task(&self, task: &PreparedV2GenesisGroupNamespace) -> Result<(), AppError> {
        match self {
            Self::Register(a) => {
                a.validate_source_only()?;
                task.retains_original(a.store(), a.source())?;
                let seed = task.cold_initializing_seed_data().ok_or_else(refused)?;
                if seed.scope_basis_data() != a.scope_basis_data()
                    || task.group() != a.group()
                    || task.signer() != a.signer()
                {
                    return Err(refused());
                }
                seed.retains_original(a.store(), a.source())
            }
            Self::Owned(a) => a.require_initializing_lineage_for_own_group_task(task),
            Self::Durable(a) => a.require_initializing_lineage_for_own_group_task(task),
            Self::Head(a) => a.require_initializing_lineage_for_own_group_task(task),
            Self::Proof(a) => a.require_retained_current_lineage_for_own_group_task(task),
            Self::Current(a) => a.require_current_lineage_for_own_group_task(task),
        }
    }
    fn require_handoff(&self, task: &PreparedV2GenesisGroupNamespace) -> Result<(), AppError> {
        match self {
            Self::Register(_) => Err(refused()),
            Self::Owned(a) => task.require_cold_initializing_owned_handoff(a),
            Self::Durable(a) => task.require_cold_initializing_owned_handoff(a.owner_admission()),
            Self::Head(a) => task.require_cold_initializing_owned_handoff(a.owner_admission()),
            Self::Proof(a) => task.require_cold_initializing_owned_handoff(a.owner_admission()),
            Self::Current(a) => task.require_cold_initializing_owned_handoff(a.owner_admission()),
        }
    }
}
struct Namespace<'task, 'writer> {
    admission: Admission<'task, 'writer>,
    task: &'task PreparedV2GenesisGroupNamespace,
    pool: MutexGuard<'task, Runtime>,
    pins: MutexGuard<'task, super::super::super::pins::Runtime>,
    association: CurrentFilenameAssociation<'task>,
    identifier: [u8; 32],
}
impl Namespace<'_, '_> {
    fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_task(self.task)?;
        self.association.require_original_data()?;
        if !self.pins.managed
            || self.pins.dirty
            || self
                .pins
                .table
                .as_ref()
                .ok_or_else(refused)?
                .contains_key(self.task.group())
            || self.pool.heads.contains_key(&self.identifier)
            || self.identifier != key(self.task.group(), self.task.signer())
        {
            return Err(refused());
        }
        match &self.admission {
            Admission::Register(_) => {
                if self.pins.pending_groups.excludes_group(self.task.group()) {
                    return Err(refused());
                }
                self.pool
                    .v2_writers
                    .require_no_live_writer(&self.identifier)
            }
            _ if self.pins.pending_groups.excludes_group(self.task.group()) => self
                .pins
                .pending_groups
                .require_same_task(self.admission.store(), self.task),
            _ => self.admission.require_handoff(self.task),
        }
    }
    fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        require_old_owner_files_absent(self.admission.store(), &self.association, self.identifier)?;
        self.validate_source_only()
    }
    fn validate_seed(
        &self,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.validate_old_files()?;
        let seed = self
            .task
            .cold_initializing_seed_data()
            .ok_or_else(refused)?;
        seed.validate_under_archive_root(self.admission.store(), self.admission.source(), root)?;
        self.admission
            .store()
            .validate_mounted_current_layout_under_archive_root(root)?;
        // All account/source/ID3 waits precede the LAST held/named group proof.
        let (path, file, id) = seed.directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(path, file, id)?;
        self.validate_source_only()
    }
}

macro_rules! namespace_role {
    ($ty:ident, $role:ident, $variant:ident, $lock:ident) => {
        pub(in crate::store) struct $ty<'task, 'writer> {
            inner: Namespace<'task, 'writer>,
            admission: &'task $role<'writer>,
        }
        impl ServerStore {
            pub(in crate::store) fn $lock<'task, 'writer>(
                &'task self,
                admission: &'task $role<'writer>,
                task: &'task PreparedV2GenesisGroupNamespace,
            ) -> Result<$ty<'task, 'writer>, AppError> {
                if !std::ptr::eq(self, admission.store()) {
                    return Err(refused());
                }
                Ok($ty {
                    inner: lock_namespace(Admission::$variant(admission), task)?,
                    admission,
                })
            }
        }
    };
}
pub(in crate::store) struct RegisteringColdInitializingNamespace<'task, 'writer> {
    inner: Namespace<'task, 'writer>,
}
impl ServerStore {
    pub(in crate::store) fn lock_registering_cold_initializing_namespace<'task, 'writer>(
        &'task self,
        admission: &'task ColdV2InitializingReadAdmission<'writer>,
        task: &'task PreparedV2GenesisGroupNamespace,
    ) -> Result<RegisteringColdInitializingNamespace<'task, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(RegisteringColdInitializingNamespace {
            inner: lock_namespace(Admission::Register(admission), task)?,
        })
    }
}

namespace_role!(
    ColdInitializingV2GenesisOwnerNamespace,
    ColdInitializingGenesisAdmission,
    Owned,
    lock_cold_initializing_owned_namespace
);
namespace_role!(
    DurableColdInitializingV2GenesisOwnerNamespace,
    DurableColdGenesisInitializingAdmission,
    Durable,
    lock_durable_cold_initializing_namespace
);
namespace_role!(
    ColdInitializingV2GenesisReadNamespace,
    ColdInitializingGenesisHeadAdmission,
    Head,
    lock_cold_initializing_head_namespace
);
namespace_role!(
    ColdRepairedV2GenesisCurrentProofNamespace,
    ColdRepairedGenesisCurrentProofAdmission,
    Proof,
    lock_cold_repaired_current_proof_namespace
);
namespace_role!(
    ColdRepairedV2GenesisCurrentNamespace,
    ColdRepairedGenesisCurrentAdmission,
    Current,
    lock_cold_repaired_current_namespace
);

macro_rules! guarded_accessors {
    ($ty:ident,$role:ident) => {
        impl<'task, 'writer> $ty<'task, 'writer> {
            pub(in crate::store) fn admission(&self) -> &'task $role<'writer> {
                self.admission
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.inner.validate_source_only()
            }
            pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
                self.inner.validate_old_files()
            }
        }
    };
}
guarded_accessors!(
    DurableColdInitializingV2GenesisOwnerNamespace,
    DurableColdGenesisInitializingAdmission
);
guarded_accessors!(
    ColdInitializingV2GenesisReadNamespace,
    ColdInitializingGenesisHeadAdmission
);
guarded_accessors!(
    ColdRepairedV2GenesisCurrentProofNamespace,
    ColdRepairedGenesisCurrentProofAdmission
);
guarded_accessors!(
    ColdRepairedV2GenesisCurrentNamespace,
    ColdRepairedGenesisCurrentAdmission
);

fn lock_namespace<'task, 'writer>(
    admission: Admission<'task, 'writer>,
    task: &'task PreparedV2GenesisGroupNamespace,
) -> Result<Namespace<'task, 'writer>, AppError> {
    admission.validate_task(task)?;
    let store = admission.store();
    task.cold_initializing_resources()?
        .validate_original(store, admission.source())?;
    let pins = store.owner_evidence.pins.lock().map_err(|_| refused())?;
    let pool = store
        .owner_evidence
        .canonical
        .lock()
        .map_err(|_| refused())?;
    let association = admission.association()?;
    let namespace = Namespace {
        admission,
        task,
        pool,
        pins,
        association,
        identifier: key(task.group(), task.signer()),
    };
    namespace.validate_old_files()?;
    Ok(namespace)
}
impl RegisteringColdInitializingNamespace<'_, '_> {
    pub(in crate::store) fn register_under_archive_root(
        &mut self,
        prepared: &mut PreparedGroupRegistration,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<RegisteredColdInitializingClaim, AppError> {
        let task = self.inner.task;
        let seed = task.cold_initializing_seed_data().ok_or_else(refused)?;
        let Some(PreparedColdGroupWitness::Initializing(witness)) = prepared.cold_witness.as_ref()
        else {
            return Err(refused());
        };
        witness.require_original(task, seed)?;
        self.inner.validate_seed(root)?;
        let Some(PreparedColdGroupWitness::Initializing(witness)) = prepared.cold_witness.take()
        else {
            unreachable!("exact witness checked before final source/ID3/name proof")
        };
        match self
            .inner
            .pins
            .pending_groups
            .install(self.inner.admission.store(), task, prepared)
        {
            Ok(()) => Ok(witness),
            Err(error) => {
                prepared.cold_witness = Some(PreparedColdGroupWitness::Initializing(witness));
                Err(error)
            }
        }
    }
}
impl ColdInitializingV2GenesisOwnerNamespace<'_, '_> {
    pub(in crate::store) fn handoff_under_archive_root(
        &mut self,
        prepared: crate::store::critical_namespaces::current_layout::group_task::cold_initializing::PreparedColdInitializingOwnerAssociation<'_, '_>,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<Option<RetiredGroupEntry>, AppError> {
        self.inner.validate_old_files()?;
        let position = self
            .inner
            .pins
            .pending_groups
            .buffer
            .as_ref()
            .and_then(|buffer| {
                buffer.slots.iter().position(|slot| {
                    slot.as_ref()
                        .is_some_and(|entry| entry.group() == self.inner.task.group())
                })
            });
        if position.is_some() {
            self.inner
                .pins
                .pending_groups
                .require_same_task(self.admission.store(), self.inner.task)?;
        } else {
            self.inner
                .task
                .require_cold_initializing_owned_handoff(self.admission)?;
        }
        self.inner.validate_seed(root)?;
        // set?take is private/infallible and performs no account/callback/last-fee drop.
        prepared.install(self.inner.task, self.admission)?;
        let entry = position.and_then(|p| {
            self.inner
                .pins
                .pending_groups
                .buffer
                .as_mut()
                .and_then(|b| b.slots.get_mut(p))
                .and_then(Option::take)
        });
        Ok(entry.map(|entry| RetiredGroupEntry { _entry: entry }))
    }
}
impl<'task, 'writer> DurableColdInitializingV2GenesisOwnerNamespace<'task, 'writer> {
    pub(in crate::store) fn require_completed_initializing_handoff(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        self.inner.admission.require_handoff(self.inner.task)
    }
    pub(in crate::store::owner_evidence::canonical) fn initializing_registration_parts(
        &mut self,
    ) -> (
        &DurableColdGenesisInitializingAdmission<'writer>,
        &mut super::super::v2_registration::Runtime,
    ) {
        (self.admission, &mut self.inner.pool.v2_writers)
    }
}
macro_rules! proof_parts {
    ($ty:ident,$role:ident) => {
        impl<'task,'writer> $ty<'task,'writer> {
            pub(in crate::store::owner_evidence::canonical) fn initializing_proof_registration_parts(&self)
                -> (&$role<'writer>, &super::super::v2_registration::Runtime) {
                (self.admission, &self.inner.pool.v2_writers)
            }
        }
    };
}
proof_parts!(
    ColdInitializingV2GenesisReadNamespace,
    ColdInitializingGenesisHeadAdmission
);
proof_parts!(
    ColdRepairedV2GenesisCurrentProofNamespace,
    ColdRepairedGenesisCurrentProofAdmission
);
proof_parts!(
    ColdRepairedV2GenesisCurrentNamespace,
    ColdRepairedGenesisCurrentAdmission
);
