//! Closed negative-only adapter for the TWO real native predecessor families.
//!
//! Only hardcoded first/next entrypoints construct these borrowed variants. A
//! common physical I/S/B proof still authenticates its own actual journal/source;
//! this enum supplies no selected parent, File/key, signer, repair or effect grant.
//! In particular Completed never becomes Clear and never reopens obsolete WC.
use super::*;
use crate::store::persistence::v2_text::OriginalV2RetainObservation;

pub(super) enum CreatePredecessorRef<'a> {
    Clear(&'a BoundV2TextCreatePredecessor),
    Completed(&'a BoundV2TextCompletedCreatePredecessor),
}

/// Both wrapper/inner borrowed call slots are named explicitly in each actual
/// I/S/B role invoice, including the unchanged first-request Clear wrappers.
pub(super) const CREATE_PREDECESSOR_REF_LAYOUT_BYTES: usize =
    4 * std::mem::size_of::<CreatePredecessorRef<'static>>();

impl CreatePredecessorRef<'_> {
    /// Pure cached original lane/request/source and domain negative. The full
    /// concrete issuer proof remains mandatory before every new finite effect.
    pub(super) fn require_same_lane_original(
        &self,
        lane: &V2TextCreateLane,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        match self {
            Self::Clear(bound) => bound.require_same_lane_original(lane, original),
            Self::Completed(bound) => {
                bound.require_same_lane_original(lane, original)?;
                bound.validate_negative()
            }
        }
    }
}
