//! Task-free historical row1/row2 negative owner locks. These roles cannot install a
//! directory task or writer, and cannot be passed to either warm metadata writer.
use super::*;
use crate::store::chat_admission::v2_current::ColdV2InitializingReadAdmission;
use crate::store::chat_admission::v2_current::{
    ColdV2CurrentReadAdmission, ColdV2GenesisScopeBasis,
};

enum ReadAdmission<'reader, 'writer> {
    Initializing(&'reader ColdV2InitializingReadAdmission<'writer>),
    Current(&'reader ColdV2CurrentReadAdmission<'writer>),
}
impl<'reader, 'writer> ReadAdmission<'reader, 'writer> {
    fn store(&self) -> &'reader ServerStore {
        match self {
            Self::Initializing(a) => a.store(),
            Self::Current(a) => a.store(),
        }
    }
    fn source(&self) -> &GenesisSourceDataRef<'_> {
        match self {
            Self::Initializing(a) => a.source(),
            Self::Current(a) => a.source(),
        }
    }
    fn group(&self) -> &[u8] {
        match self {
            Self::Initializing(a) => a.group(),
            Self::Current(a) => a.group(),
        }
    }
    fn signer(&self) -> &[u8; 32] {
        match self {
            Self::Initializing(a) => a.signer(),
            Self::Current(a) => a.signer(),
        }
    }
    fn basis(&self) -> &ColdV2GenesisScopeBasis {
        match self {
            Self::Initializing(a) => a.scope_basis_data(),
            Self::Current(a) => a.scope_basis_data(),
        }
    }
    fn validate_source_only(&self) -> Result<(), AppError> {
        match self {
            Self::Initializing(a) => a.validate_source_only(),
            Self::Current(a) => a.validate_source_only(),
        }
    }
    fn association(&self) -> Result<CurrentFilenameAssociation<'reader>, AppError> {
        match self {
            Self::Initializing(a) => {
                let admission: &'reader ColdV2InitializingReadAdmission<'writer> = a;
                admission.current_filename_association()
            }
            Self::Current(a) => {
                let admission: &'reader ColdV2CurrentReadAdmission<'writer> = a;
                admission.current_filename_association()
            }
        }
    }
}

struct ColdOwnerReadNamespace<'reader, 'writer> {
    // The actual role borrow keeps admission held. Release canonical before pins; neither
    // guard owns a refundable fee, task sender or mutable runtime/source graph.
    admission: ReadAdmission<'reader, 'writer>,
    pool: MutexGuard<'reader, Runtime>,
    pins: MutexGuard<'reader, super::super::super::pins::Runtime>,
    association: CurrentFilenameAssociation<'reader>,
    identifier: [u8; 32],
}
impl ColdOwnerReadNamespace<'_, '_> {
    fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_source_only()?;
        self.association.require_original_data()?;
        let group = self.admission.group();
        if !self.pins.managed
            || self.pins.dirty
            || self
                .pins
                .table
                .as_ref()
                .ok_or_else(refused)?
                .contains_key(group)
            || self.pins.pending_groups.excludes_group(group)
            || self.pool.heads.contains_key(&self.identifier)
            || self.identifier != key(group, self.admission.signer())
        {
            return Err(refused());
        }
        // A strong but cancelled registration is still draining. Do not upgrade/drop an
        // arbitrary Life here: that could run its last fee destructor under canonical.
        self.pool
            .v2_writers
            .require_no_live_writer(&self.identifier)
    }
    fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        require_old_owner_files_absent(self.admission.store(), &self.association, self.identifier)?;
        self.validate_source_only()
    }
}

/// Read-only historical Initializing1 with no owner. It carries no Init1 storage permit.
pub(in crate::store) struct ColdV2InitializingOwnerReadNamespace<'reader, 'writer> {
    inner: ColdOwnerReadNamespace<'reader, 'writer>,
}
/// Read-only historical Current2 with no owner. It cannot install or renew a writer.
pub(in crate::store) struct ColdV2CurrentOwnerReadNamespace<'reader, 'writer> {
    inner: ColdOwnerReadNamespace<'reader, 'writer>,
}
macro_rules! accessors {
    ($ty:ident) => {
        impl $ty<'_, '_> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.inner.admission.store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.inner.admission.source()
            }
            pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
                self.inner.admission.basis()
            }
            /// Cached negatives only; actual old directory/name probes are separate.
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.inner.validate_source_only()
            }
            pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
                self.inner.validate_old_files()
            }
        }
    };
}
accessors!(ColdV2InitializingOwnerReadNamespace);
accessors!(ColdV2CurrentOwnerReadNamespace);

fn lock_read_namespace<'reader, 'writer>(
    admission: ReadAdmission<'reader, 'writer>,
) -> Result<ColdOwnerReadNamespace<'reader, 'writer>, AppError> {
    admission.validate_source_only()?;
    let store = admission.store();
    let pins = store.owner_evidence.pins.lock().map_err(|_| refused())?;
    let pool = store
        .owner_evidence
        .canonical
        .lock()
        .map_err(|_| refused())?;
    let identifier = key(admission.group(), admission.signer());
    let association = admission.association()?;
    let result = ColdOwnerReadNamespace {
        admission,
        pool,
        pins,
        association,
        identifier,
    };
    result.validate_old_files()?;
    Ok(result)
}
impl ServerStore {
    pub(in crate::store) fn lock_cold_initializing_v2_owner_namespace<'reader, 'writer>(
        &'reader self,
        admission: &'reader ColdV2InitializingReadAdmission<'writer>,
    ) -> Result<ColdV2InitializingOwnerReadNamespace<'reader, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(ColdV2InitializingOwnerReadNamespace {
            inner: lock_read_namespace(ReadAdmission::Initializing(admission))?,
        })
    }
    pub(in crate::store) fn lock_cold_current_v2_owner_namespace<'reader, 'writer>(
        &'reader self,
        admission: &'reader ColdV2CurrentReadAdmission<'writer>,
    ) -> Result<ColdV2CurrentOwnerReadNamespace<'reader, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(ColdV2CurrentOwnerReadNamespace {
            inner: lock_read_namespace(ReadAdmission::Current(admission))?,
        })
    }
}
