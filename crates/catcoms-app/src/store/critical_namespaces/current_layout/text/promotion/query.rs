//! Narrow same-original physical-resource checks for the sibling actual query admission.
//! This exposes no OS lease adopter or generic Current/selected-root converter.
use super::*;

impl PreparedV2TextPhysicalLease {
    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_token_query_capture_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }

    pub(in crate::store::critical_namespaces::current_layout::text) fn require_token_query_capture_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }

    pub(in crate::store::critical_namespaces::current_layout::text) fn validate_accepted_query_capture_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_under_current_guard(guard)
    }
    pub(in crate::store::critical_namespaces::current_layout::text) fn require_accepted_query_capture_negative(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(guard)
    }
}
