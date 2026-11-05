//! Group-wide negative row/owner queries. No Current, absence durability or source authority
//! follows from these in-memory comparisons; concrete callers retain their own original fences.
use super::*;
use std::ops::Bound::Included;

/// Requested bound-key backing plus its fixed owner/cursor layouts. Callers pay before `new`
/// and keep that independent actual fee through both Vecs; this is not a whole-table invoice.
pub(in crate::store) const GROUP_BOUNDS_ALLOCATION_BYTES: usize = 2 * 256
    + std::mem::size_of::<ChatAdmissionGroupBounds>()
    + std::mem::size_of::<crate::store::critical_namespaces::read_pool::CatalogReadReservation>()
    + std::mem::size_of::<std::collections::btree_map::Range<'static, ChatAdmissionKey, Record>>()
    + std::mem::size_of::<
        std::collections::btree_map::Range<'static, ChatAdmissionKey, AdmissionOwner>,
    >();

/// Same complete group with the closed signer endpoints. Construct OUTSIDE all host writers.
/// There is no serialized constructor, mutation or iterator/source setter; limits/data alone
/// cannot claim that a Store has loaded a complete clean admission inventory.
pub(in crate::store) struct ChatAdmissionGroupBounds {
    lower: ChatAdmissionKey,
    upper: ChatAdmissionKey,
}
impl std::fmt::Debug for ChatAdmissionGroupBounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAdmissionGroupBounds")
            .finish_non_exhaustive()
    }
}
impl ChatAdmissionGroupBounds {
    pub(in crate::store) fn new(group: &[u8]) -> Result<Self, AppError> {
        Ok(Self {
            lower: ChatAdmissionKey::new(group, [0; 32])?,
            upper: ChatAdmissionKey::new(group, [255; 32])?,
        })
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.lower.group()
    }

    /// First-genesis vacancy requires the actual complete, managed, CLEAN table. Even Retired
    /// rows and orphan live-owner bookkeeping are conflicts. Look up one endpoint range in
    /// each non-evicting tree: no lifetime group/table scan or temporary key allocation.
    pub(super) fn require_absent(&self, runtime: &Runtime) -> Result<(), AppError> {
        if !runtime.managed || runtime.dirty {
            return Err(refused());
        }
        let table = runtime.table.as_ref().ok_or_else(refused)?;
        if table
            .range((Included(&self.lower), Included(&self.upper)))
            .next()
            .is_some()
            || runtime
                .owners
                .range((Included(&self.lower), Included(&self.upper)))
                .next()
                .is_some()
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Negative companion exclusion only. A SAME dirty latest table may be inspected for
    /// exact Initializing/Current barrier repair; this does not assert that own row is present,
    /// matches any snapshot/phase, is saved or authorizes an effect. Actual callers must make
    /// those stronger checks. There can be only one own key, so two observations suffice to
    /// detect ANY foreign key without scanning a group's lifetime member history.
    // Future positive scope consumer is still undeclared. Keep this negative DATA query in
    // tests until that actual consumer lands with its own original/phase/barrier gates.
    #[cfg(test)]
    pub(super) fn require_no_foreign(
        &self,
        runtime: &Runtime,
        own: &ChatAdmissionKey,
    ) -> Result<(), AppError> {
        if !runtime.managed || own.group() != self.group() {
            return Err(refused());
        }
        let table = runtime.table.as_ref().ok_or_else(refused)?;
        if table
            .range((Included(&self.lower), Included(&self.upper)))
            .take(2)
            .any(|(key, _)| key != own)
            || runtime
                .owners
                .range((Included(&self.lower), Included(&self.upper)))
                .take(2)
                .any(|(key, _)| key != own)
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
