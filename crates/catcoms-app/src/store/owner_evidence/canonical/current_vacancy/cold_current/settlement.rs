//! Readonly canonical-family loan for a NEW settlement/settled typed owner guard. This is
//! not a registration/install seam and exposes nothing outside the canonical family.
use super::*;
impl ColdOwnedV2CurrentOwnerNamespace<'_, '_> {
    pub(in crate::store::owner_evidence::canonical) fn settlement_registration_runtime(
        &self,
    ) -> &crate::store::owner_evidence::canonical::v2_registration::Runtime {
        &self.inner.pool.v2_writers
    }
}
