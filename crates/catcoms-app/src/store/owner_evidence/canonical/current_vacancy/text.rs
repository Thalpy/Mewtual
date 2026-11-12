//! Readonly existing-owner namespace for selected operational text heads.
//!
//! The original three ownership families remain concrete. Reusing their cached pins/old
//! namespace exclusion must not call a genesis-only Root/head validator after promotion.
//! There is no registration, owner replacement, task adoption or raw Runtime escape here.
use super::settlement::{lock_existing, ExistingOwner};
use super::*;
use crate::store::chat_admission::v2_current::V2TextCurrentAdmission;

pub(in crate::store) struct V2TextOwnerNamespace<'guard, 'writer> {
    inner: ExistingOwner<'guard, 'writer>,
    admission: &'guard V2TextCurrentAdmission<'writer>,
}

/// The new text metadata invoice pays these borrowed guard layouts in addition to the
/// existing original owners. No new old-head map, checkpoint budget or heap is created.
pub(in crate::store) const V2_TEXT_OWNER_NAMESPACE_BYTES: usize =
    std::mem::size_of::<V2TextOwnerNamespace<'static, 'static>>();

impl<'guard, 'writer> V2TextOwnerNamespace<'guard, 'writer> {
    pub(in crate::store) fn scope_admission(&self) -> &'guard V2TextCurrentAdmission<'writer> {
        self.admission
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_source_only()?;
        self.inner.validate_source_only()
    }
    /// Reopened exact old-owner files/directory and borrowed complete filename association
    /// are repeated after waits. This does not select or interpret operational O/H bytes.
    pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        self.inner.validate_old_files()?;
        self.validate_source_only()
    }
    /// Only the canonical kernel child sees its OWN readonly writer generation map.
    /// Returning a raw Scope Runtime or mutable registration map would violate this seam.
    pub(in crate::store::owner_evidence::canonical) fn text_registration_parts(
        &self,
    ) -> (&super::super::v2_registration::Runtime, &[u8; 32]) {
        self.inner.registration_parts()
    }

    /// Canonical-only immutable exact reader-slot validation; no runtime escapes Store.
    pub(in crate::store::owner_evidence::canonical) fn text_lane_runtime(
        &self,
    ) -> &super::super::text_lane::TextLaneRuntime {
        self.inner.text_lane_runtime()
    }

    /// Only canonical's negative lane kernel can mutate its OWN fixed active-work slots.
    /// This is independent of (and cannot mutate) the installed-writer generation map.
    pub(in crate::store::owner_evidence::canonical) fn text_lane_runtime_mut(
        &mut self,
    ) -> &mut super::super::text_lane::TextLaneRuntime {
        self.inner.text_lane_runtime_mut()
    }
}

impl ServerStore {
    pub(in crate::store) fn lock_v2_text_owner_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard V2TextCurrentAdmission<'writer>,
    ) -> Result<V2TextOwnerNamespace<'guard, 'writer>, AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        // Caller still owns the SAME admission writer. This acquires pins -> canonical,
        // preserves the original role's task/OnceLock proof and does no Root relock/IO.
        let namespace = V2TextOwnerNamespace {
            inner: lock_existing(admission.original_read())?,
            admission,
        };
        namespace.validate_old_files()?;
        Ok(namespace)
    }
}

impl std::fmt::Debug for V2TextOwnerNamespace<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextOwnerNamespace")
            .finish_non_exhaustive()
    }
}
