//! SAME original physical custody for S. This thin negative seam cannot adopt an
//! OS lease or convert a signature record, signing guard or parsed WI into authority.
use super::*;

impl PreparedV2TextPhysicalLease {
    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_signature_write_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }

    pub(in crate::store::critical_namespaces::current_layout::text) fn require_signature_write_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }
}
