//! Complete-journal/no-row claims and their exact same-owner born successor. Only these
//! concrete admission roles borrow the existing pending16 slots; no second registry exists.
use super::*;
use crate::store::chat_admission::v2_current::{
    BornV2GenesisAdmission, ColdV2GenesisVacantReadAdmission, DurableBornV2GenesisRow1Admission,
};
use crate::store::critical_namespaces::current_layout::group_task::cold_preinitializing::PreparedBornGroupSuccessor;
use crate::store::critical_namespaces::current_layout::OriginalV2GenesisGroupObservation;
use crate::store::persistence::v2_genesis_activation::pre_initializing::ColdV2PreInitializingSeed;

/// Prebuilt outside writers; issued only after the original slot actually transfers.
pub(in crate::store) struct RegisteredPreInitializingClaim {
    seed: Arc<ColdV2PreInitializingSeed>,
    original: OriginalV2GenesisGroupObservation,
}
impl RegisteredPreInitializingClaim {
    pub(super) fn prepare(task: &PreparedV2GenesisGroupNamespace) -> Result<Self, AppError> {
        Ok(Self {
            seed: task
                .pre_initializing_seed_data()
                .ok_or_else(refused)?
                .clone(),
            original: task.observe_original(),
        })
    }
    pub(in crate::store) fn require_original(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
        seed: &Arc<ColdV2PreInitializingSeed>,
    ) -> Result<(), AppError> {
        self.original.require_same_original(task)?;
        if !Arc::ptr_eq(&self.seed, seed) {
            return Err(refused());
        }
        Ok(())
    }
}

enum Admission<'guard, 'writer> {
    Vacant(&'guard ColdV2GenesisVacantReadAdmission<'writer>),
    Born(&'guard BornV2GenesisAdmission<'writer>),
    Durable(&'guard DurableBornV2GenesisRow1Admission<'writer>),
}
impl<'guard, 'writer> Admission<'guard, 'writer> {
    fn store(&self) -> &'guard ServerStore {
        match self {
            Self::Vacant(a) => a.store(),
            Self::Born(a) => a.store(),
            Self::Durable(a) => a.store(),
        }
    }
    fn source(&self) -> &GenesisSourceDataRef<'_> {
        match self {
            Self::Vacant(a) => a.source(),
            Self::Born(a) => a.source(),
            Self::Durable(a) => a.source(),
        }
    }
    fn group(&self) -> &[u8] {
        match self {
            Self::Vacant(a) => a.group(),
            Self::Born(a) => a.group(),
            Self::Durable(a) => a.group(),
        }
    }
    fn signer(&self) -> &[u8; 32] {
        match self {
            Self::Vacant(a) => a.signer(),
            Self::Born(a) => a.signer(),
            Self::Durable(a) => a.signer(),
        }
    }
    fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::Vacant(a) => a.validate_source_only(),
            Self::Born(a) => a.validate_source_only(),
            Self::Durable(a) => a.validate_source_only(),
        }
    }
    fn association(&self) -> Result<CurrentFilenameAssociation<'guard>, AppError> {
        match self {
            Self::Vacant(a) => {
                let a: &'guard ColdV2GenesisVacantReadAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Born(a) => {
                let a: &'guard BornV2GenesisAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Durable(a) => {
                let a: &'guard DurableBornV2GenesisRow1Admission<'writer> = a;
                a.current_filename_association()
            }
        }
    }
    fn validate_task(&self, task: &PreparedV2GenesisGroupNamespace) -> Result<(), AppError> {
        self.validate()?;
        task.retains_original(self.store(), self.source())?;
        if task.group() != self.group() || task.signer() != self.signer() {
            return Err(refused());
        }
        match self {
            Self::Vacant(a) => {
                let seed = task.pre_initializing_seed_data().ok_or_else(refused)?;
                if seed.vacancy_basis_data() != a.vacancy_basis_data() {
                    return Err(refused());
                }
                seed.retains_original(self.store(), self.source())
            }
            Self::Born(a) => task.require_born_v2_genesis_scope_owner(a),
            Self::Durable(a) => task.require_born_v2_genesis_scope_owner(a.born_admission()),
        }
    }
}

struct Namespace<'guard, 'writer> {
    admission: Admission<'guard, 'writer>,
    task: Option<&'guard PreparedV2GenesisGroupNamespace>,
    // Release canonical before pins. Neither guard owns a sender or refundable payment.
    pool: MutexGuard<'guard, Runtime>,
    pins: MutexGuard<'guard, super::super::super::pins::Runtime>,
    association: CurrentFilenameAssociation<'guard>,
    identifier: [u8; 32],
}
impl Namespace<'_, '_> {
    fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate()?;
        self.association.require_original_data()?;
        if !self.pins.managed
            || self.pins.dirty
            || self
                .pins
                .table
                .as_ref()
                .ok_or_else(refused)?
                .contains_key(self.admission.group())
            || self.pool.heads.contains_key(&self.identifier)
            || self.identifier != key(self.admission.group(), self.admission.signer())
        {
            return Err(refused());
        }
        self.pool
            .v2_writers
            .require_no_live_writer(&self.identifier)?;
        match self.task {
            None => {
                if self
                    .pins
                    .pending_groups
                    .excludes_group(self.admission.group())
                {
                    return Err(refused());
                }
                Ok(())
            }
            Some(task) => {
                self.admission.validate_task(task)?;
                if self.pins.pending_groups.excludes_group(task.group()) {
                    // The slot still has its OLD kind until successor.set. Never inspect a
                    // successor through this branch after that commit edge.
                    self.pins
                        .pending_groups
                        .require_same_task(self.admission.store(), task)
                } else {
                    match &self.admission {
                        Admission::Born(a) => task.require_born_v2_genesis_handoff(a),
                        Admission::Durable(a) => {
                            task.require_born_v2_genesis_handoff(a.born_admission())
                        }
                        Admission::Vacant(_) => Err(refused()),
                    }
                }
            }
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
        let task = self.task.ok_or_else(refused)?;
        task.pre_initializing_seed_data()
            .ok_or_else(refused)?
            .validate_under_archive_root(self.admission.store(), self.admission.source(), root)?;
        self.validate_source_only()
    }
}
fn lock<'guard, 'writer>(
    admission: Admission<'guard, 'writer>,
    task: Option<&'guard PreparedV2GenesisGroupNamespace>,
) -> Result<Namespace<'guard, 'writer>, AppError> {
    admission.validate()?;
    if let Some(task) = task {
        admission.validate_task(task)?;
    }
    let store = admission.store();
    let pins = store.owner_evidence.pins.lock().map_err(|_| refused())?;
    let pool = store
        .owner_evidence
        .canonical
        .lock()
        .map_err(|_| refused())?;
    let association = admission.association()?;
    let result = Namespace {
        identifier: key(admission.group(), admission.signer()),
        admission,
        task,
        pool,
        pins,
        association,
    };
    result.validate_old_files()?;
    Ok(result)
}

pub(in crate::store) struct ColdV2VacantOwnerReadNamespace<'guard, 'writer> {
    inner: Namespace<'guard, 'writer>,
    admission: &'guard ColdV2GenesisVacantReadAdmission<'writer>,
}
pub(in crate::store) struct RegisteringPreInitializingNamespace<'guard, 'writer> {
    inner: Namespace<'guard, 'writer>,
}
pub(in crate::store) struct BornV2GenesisOwnerNamespace<'guard, 'writer> {
    inner: Namespace<'guard, 'writer>,
    admission: &'guard BornV2GenesisAdmission<'writer>,
}
pub(in crate::store) struct DurableBornV2GenesisOwnerNamespace<'guard, 'writer> {
    inner: Namespace<'guard, 'writer>,
    admission: &'guard DurableBornV2GenesisRow1Admission<'writer>,
}
macro_rules! accessors {
    ($ty:ident) => {
        impl<'guard, 'writer> $ty<'guard, 'writer> {
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.inner.validate_source_only()
            }
            pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
                self.inner.validate_old_files()
            }
        }
    };
}
accessors!(ColdV2VacantOwnerReadNamespace);
accessors!(BornV2GenesisOwnerNamespace);
accessors!(DurableBornV2GenesisOwnerNamespace);

impl<'guard, 'writer> ColdV2VacantOwnerReadNamespace<'guard, 'writer> {
    pub(in crate::store) fn admission(&self) -> &'guard ColdV2GenesisVacantReadAdmission<'writer> {
        self.admission
    }
}
impl<'guard, 'writer> DurableBornV2GenesisOwnerNamespace<'guard, 'writer> {
    pub(in crate::store) fn admission(&self) -> &'guard DurableBornV2GenesisRow1Admission<'writer> {
        self.admission
    }
}

impl ServerStore {
    pub(in crate::store) fn lock_cold_vacant_owner_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard ColdV2GenesisVacantReadAdmission<'writer>,
    ) -> Result<ColdV2VacantOwnerReadNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(ColdV2VacantOwnerReadNamespace {
            inner: lock(Admission::Vacant(admission), None)?,
            admission,
        })
    }
    pub(in crate::store) fn lock_pre_initializing_birth_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard ColdV2GenesisVacantReadAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<ColdV2VacantOwnerReadNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(ColdV2VacantOwnerReadNamespace {
            inner: lock(Admission::Vacant(admission), Some(task))?,
            admission,
        })
    }
    pub(in crate::store) fn lock_registering_pre_initializing_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard ColdV2GenesisVacantReadAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<RegisteringPreInitializingNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        // Registration proves no existing group debt; the original task is checked before
        // acquiring this task-free bundle, then again before the exact slot transfer.
        let inner = lock(Admission::Vacant(admission), None)?;
        inner.admission.validate_task(task)?;
        Ok(RegisteringPreInitializingNamespace { inner })
    }
    pub(in crate::store) fn lock_born_v2_genesis_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard BornV2GenesisAdmission<'writer>,
    ) -> Result<BornV2GenesisOwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(BornV2GenesisOwnerNamespace {
            inner: lock(Admission::Born(admission), Some(admission.task()))?,
            admission,
        })
    }
    pub(in crate::store) fn lock_durable_born_v2_genesis_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard DurableBornV2GenesisRow1Admission<'writer>,
    ) -> Result<DurableBornV2GenesisOwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(DurableBornV2GenesisOwnerNamespace {
            inner: lock(Admission::Durable(admission), Some(admission.task()))?,
            admission,
        })
    }
}
impl RegisteringPreInitializingNamespace<'_, '_> {
    pub(in crate::store) fn register_under_archive_root(
        &mut self,
        task: &PreparedV2GenesisGroupNamespace,
        prepared: &mut PreparedGroupRegistration,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<RegisteredPreInitializingClaim, AppError> {
        self.inner.admission.validate_task(task)?;
        let seed = task.pre_initializing_seed_data().ok_or_else(refused)?;
        let Some(PreparedColdGroupWitness::PreInitializing(witness)) =
            prepared.cold_witness.as_ref()
        else {
            return Err(refused());
        };
        witness.require_original(task, seed)?;
        self.inner.validate_old_files()?;
        seed.validate_under_archive_root(
            self.inner.admission.store(),
            self.inner.admission.source(),
            root,
        )?;
        self.inner.validate_source_only()?;
        let Some(PreparedColdGroupWitness::PreInitializing(witness)) = prepared.cold_witness.take()
        else {
            unreachable!("original variant checked before proof")
        };
        match self
            .inner
            .pins
            .pending_groups
            .install(self.inner.admission.store(), task, prepared)
        {
            Ok(()) => Ok(witness),
            Err(error) => {
                prepared.cold_witness = Some(PreparedColdGroupWitness::PreInitializing(witness));
                Err(error)
            }
        }
    }
}
impl BornV2GenesisOwnerNamespace<'_, '_> {
    pub(in crate::store) fn handoff_under_archive_root(
        &mut self,
        prepared: PreparedBornGroupSuccessor<'_, '_>,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<Option<RetiredGroupEntry>, AppError> {
        self.inner.validate_seed(root)?;
        let task = self.admission.task();
        let position = self
            .inner
            .pins
            .pending_groups
            .buffer
            .as_ref()
            .and_then(|b| {
                b.slots
                    .iter()
                    .position(|slot| slot.as_ref().is_some_and(|e| e.group() == task.group()))
            });
        if position.is_some() {
            self.inner
                .pins
                .pending_groups
                .require_same_task(self.admission.store(), task)?;
        } else {
            task.require_born_v2_genesis_handoff(self.admission)?;
        }
        self.admission
            .born_repair_seed()
            .ok_or_else(refused)?
            .validate_under_archive_root(self.admission.store(), self.admission.source(), root)?;
        self.inner.validate_source_only()?;
        // ALL old-kind checks precede set. Only an infallible original slot move follows;
        // the returned paid DATA drains after AD/pins/canonical/Root release.
        prepared.install(task, self.admission)?;
        let entry = position.and_then(|p| {
            self.inner
                .pins
                .pending_groups
                .buffer
                .as_mut()
                .and_then(|b| b.slots.get_mut(p))
                .and_then(Option::take)
        });
        Ok(entry.map(|_entry| RetiredGroupEntry { _entry }))
    }
}
