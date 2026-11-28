//! Per-case evidence for a public signed review stream. It commits to
//! revisions and the exact original while keeping private records local.
use super::*;

pub(super) const REVIEW_CASE_STEP_BYTES: usize = 3 * 32 + 1;
const ORIGINAL: &str = "mewtual/chat/removed-post-public-original/v1";

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReviewCaseStep {
    case: ReviewCaseId,
    original: [u8; 32],
    choice: ReviewDecision,
    previous: Option<OwnerBodyId>,
}

catcoms_rt::redacted_debug! { ReviewCaseStep; }

impl ReviewCaseStep {
    pub fn from_decision(decision: &ExactReviewDecisionBody) -> Result<Self, ChatError> {
        let case = decision.case_id();
        let original = decision.original_data();
        let mut commitment = blake3::Hasher::new_derive_key(ORIGINAL);
        commitment.update(case.as_bytes());
        commitment.update(original.digest.as_bytes());
        commitment.update(original.remove_header.as_bytes());
        commitment.update(&original.remove_epoch.to_be_bytes());
        let spec = decision.decision().spec();
        if spec.previous.is_some_and(|id| id.as_bytes() == &[0; 32]) {
            return Err(ChatError::Malformed);
        }
        Ok(Self {
            case,
            original: *commitment.finalize().as_bytes(),
            choice: spec.decision,
            previous: spec.previous,
        })
    }

    pub fn case_data(&self) -> ReviewCaseId {
        self.case
    }

    pub fn choice_data(&self) -> ReviewDecision {
        self.choice
    }

    pub fn require_decision(&self, body: &ExactReviewDecisionBody) -> Result<(), ChatError> {
        if *self != Self::from_decision(body)? {
            return Err(ChatError::Scope);
        }
        Ok(())
    }

    /// The caller supplies the case tip selected under the same pinned
    /// stream. An absent, stale or different case tip grants no approval.
    pub fn verify_transition(
        &self,
        previous: Option<(OwnerBodyId, Self)>,
    ) -> Result<(), ChatError> {
        if self.previous != previous.as_ref().map(|(id, _)| *id) {
            return Err(ChatError::Dependency);
        }
        if previous
            .is_some_and(|(_, prior)| prior.case != self.case || prior.original != self.original)
        {
            return Err(ChatError::Scope);
        }
        self.choice
            .require_transition(previous.map(|(_, prior)| prior.choice))
    }

    pub(super) fn encode(&self) -> [u8; REVIEW_CASE_STEP_BYTES] {
        let mut frame = [0; REVIEW_CASE_STEP_BYTES];
        frame[..32].copy_from_slice(self.case.as_bytes());
        frame[32..64].copy_from_slice(&self.original);
        let previous = self.previous.map_or([0; 32], |id| *id.as_bytes());
        frame[64..96].copy_from_slice(&previous);
        frame[96] = self.choice.tag();
        frame
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        let frame: &[u8; REVIEW_CASE_STEP_BYTES] =
            bytes.try_into().map_err(|_| ChatError::Malformed)?;
        let case: [u8; 32] = frame[..32].try_into().map_err(|_| ChatError::Malformed)?;
        let original: [u8; 32] = frame[32..64].try_into().map_err(|_| ChatError::Malformed)?;
        let previous: [u8; 32] = frame[64..96].try_into().map_err(|_| ChatError::Malformed)?;
        let choice = ReviewDecision::from_tag(frame[96])?;
        if case == [0; 32]
            || original == [0; 32]
            || (previous == [0; 32] && choice == ReviewDecision::Reopen)
        {
            return Err(ChatError::Malformed);
        }
        Ok(Self {
            case: ReviewCaseId::from_bytes(case),
            original,
            choice,
            previous: (previous != [0; 32]).then_some(OwnerBodyId::from_bytes(previous)),
        })
    }
}
