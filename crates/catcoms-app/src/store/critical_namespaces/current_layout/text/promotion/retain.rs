//! Narrow actual same-original physical custody checks for A stage/commit admissions.
//! Neither method selects a source, adopts a lease, creates a task or grants write access.
use super::*;

impl PreparedV2TextPhysicalLease {
    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_retain_write_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }
    pub(in crate::store::critical_namespaces::current_layout::text) fn require_retain_write_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }
}
