//! SAME original physical custody for B. No accepted locator or historical receipt
//! can adopt a lease or substitute for the actual typed B stage/commit proof.
use super::*;

impl PreparedV2TextPhysicalLease {
    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_accepted_write_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }

    pub(in crate::store::critical_namespaces::current_layout::text) fn require_accepted_write_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }
}
