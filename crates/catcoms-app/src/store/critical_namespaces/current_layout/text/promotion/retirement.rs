//! SAME original physical custody for the distinct WD and WC finite effects. No
//! parsed deletion list, historical receipt or root counter can adopt this lease.
use super::*;

impl PreparedV2TextPhysicalLease {
    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_retirement_write_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }
    pub(in crate::store::critical_namespaces::current_layout::text) fn require_retirement_write_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }
}
