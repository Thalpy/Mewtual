//! Exact installed-generation proof for the actual text Current namespace.
//!
//! This is a readonly call into the SAME existing map/kernel. It cannot create a sender,
//! register/renew a writer, upgrade WriterLife, retire a slot or infer a new ownership role.
use super::*;
use crate::store::owner_evidence::canonical::current_vacancy::text::V2TextOwnerNamespace;

impl V2TextOwnerNamespace<'_, '_> {
    pub(in crate::store) fn require_installed_text_writer(
        &self,
        original: &V2WriterObservation,
    ) -> Result<(), AppError> {
        self.validate_source_only()?;
        let (runtime, key) = self.text_registration_parts();
        runtime.require_installed_observation(key, original)?;
        self.validate_source_only()
    }
}
