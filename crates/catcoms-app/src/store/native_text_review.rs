//! Exact caller input for one recovered founder's initial review intent.
use super::persistence::v2_review_hold::HeldCase;
use catcoms_replication::chat::{
    owner_bodies::{ExactReviewOriginal, ReviewCaseId, ReviewDecision},
    owner_header::OwnerBodyId,
    ActionId,
};

pub struct NativeTextInitialReviewRequest {
    pub(super) channel: u128,
    pub(super) case: HeldCase,
    pub(super) intent: [u8; 16],
    pub(super) decision: ReviewDecision,
    pub(super) previous: Option<OwnerBodyId>,
}

catcoms_rt::redacted_debug! { NativeTextInitialReviewRequest; }

impl NativeTextInitialReviewRequest {
    /// Expected case coordinates from the actual founder audit. Native
    /// still lends the original row, scoped UI command and owned runtime.
    pub fn new(
        channel: u128,
        (id, action, exact): (ReviewCaseId, ActionId, ExactReviewOriginal),
        intent: [u8; 16],
        decision: ReviewDecision,
    ) -> Self {
        Self {
            channel,
            case: HeldCase {
                id,
                action,
                original: exact.digest,
                removal: (exact.remove_header, exact.remove_epoch),
            },
            intent,
            decision,
            previous: None,
        }
    }

    /// Match the exact case revision that the founder saw before making
    /// a follow-up decision. A separate trusted journal lookup proves it.
    pub fn for_successor(mut self, previous: OwnerBodyId) -> Self {
        self.previous = Some(previous);
        self
    }
}
