//! Narrow same-original physical custody checks for actual unsigned-I admissions.
//! No OS lease adopter, caller-selected source, node or generic sign/Current role.
use super::*;

impl PreparedV2TextPhysicalLease {
    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_intent_write_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }
    pub(in crate::store::critical_namespaces::current_layout::text) fn require_intent_write_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }
}
