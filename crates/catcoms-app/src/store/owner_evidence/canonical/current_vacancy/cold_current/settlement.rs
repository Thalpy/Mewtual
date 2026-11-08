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

impl ColdOwnedV2CurrentOwnerNamespace<'_, '_> {
    pub(in crate::store::owner_evidence::canonical) fn text_lane_runtime(
        &self,
    ) -> &crate::store::owner_evidence::canonical::text_lane::TextLaneRuntime {
        &self.inner.pool.text_lanes
    }
    /// Family-private negative-lane storage only; no mutable writer map escapes.
    pub(in crate::store::owner_evidence::canonical) fn text_lane_runtime_mut(
        &mut self,
    ) -> &mut crate::store::owner_evidence::canonical::text_lane::TextLaneRuntime {
        &mut self.inner.pool.text_lanes
    }
}
