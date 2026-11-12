//! WIP borrowed adapter for the real filename index. Declare only with Factory's actual
//! current_lineage and canonical group guards. Nothing here selects an empty/default index,
//! exposes Runtime, scans directories, creates authority or pays the physical read scratch.
use super::*;
use crate::store::chat_admission::current_lineage::CurrentFilenameAssociation;

impl VacantV2GenesisAdmission<'_> {
    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<CurrentFilenameAssociation<'_>, AppError> {
        self.validate_source_only()?;
        self.runtime.current_filename_association(self.store)
    }
}

impl V2CurrentAdmission<'_> {
    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<CurrentFilenameAssociation<'_>, AppError> {
        self.validate_source_only()?;
        self.runtime.current_filename_association(self.store)
    }
}

impl V2InitializingAdmission<'_> {
    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<CurrentFilenameAssociation<'_>, AppError> {
        // Keep the actual clean Init1 storage permit in the check, rather than treating its
        // general current-admission member as sufficient for an initializing writer role.
        self.validate_source_only()?;
        self.admission.current_filename_association()
    }
}
