//! Exact review DATA. Only the selected signed governing decision can supply later REVIEWED
//! admission; it never appends to a frozen log or proves accepted-before-removal. No positive
//! ordinary/projection/Delete-approval witness or current-case selection is created here.
use super::*;

const DOMAIN: &[u8] = b"mewtual/chat/review-decision/v2\0";
const CASE_DOMAIN: &[u8] = b"mewtual/chat/review-case/v2\0";
pub const MAX_REVIEW_BODY_BYTES: usize =
    4 + DOMAIN.len() + 4 + MAX_ACCEPTANCE_RECORD_BYTES + 4 + 16 + 1 + 1 + 36;

/// A stable exact target label, distinct from ActionId/owner-body IDs. Raw construction grants
/// neither case existence, selected head, read permission nor a positive review decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReviewCaseId([u8; 32]);
impl ReviewCaseId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Approve,
    Reject,
    Reopen,
}
impl ReviewDecision {
    fn tag(self) -> u8 {
        match self {
            Self::Approve => 1,
            Self::Reject => 2,
            Self::Reopen => 3,
        }
    }
    fn from_tag(tag: u8) -> Result<Self, ChatError> {
        match tag {
            1 => Ok(Self::Approve),
            2 => Ok(Self::Reject),
            3 => Ok(Self::Reopen),
            _ => Err(ChatError::Malformed),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSpec {
    /// Target binding only. It need not have been accepted into the frozen log; that is the
    /// unique late-post case being reviewed. A raw target record is not positive evidence.
    pub target: AcceptanceRecord,
    /// Exact retry identity input, not authority. Zero/repeated nonces are not globally unique;
    /// the actual producer matches its retained original request/result before new work.
    pub intent: [u8; 16],
    pub decision: ReviewDecision,
    pub previous: Option<OwnerBodyId>,
}
impl ReviewSpec {
    fn validate(&self) -> Result<(), ChatError> {
        if self.decision == ReviewDecision::Reopen && self.previous.is_none() {
            Err(ChatError::Dependency)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReviewDecisionBody {
    spec: ReviewSpec,
}
impl std::fmt::Debug for ReviewDecisionBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewDecisionBody")
            .field("case", &self.case_id())
            .field("decision", &self.spec.decision)
            .finish_non_exhaustive()
    }
}
impl ReviewDecisionBody {
    pub fn new(spec: ReviewSpec) -> Result<Self, ChatError> {
        spec.validate()?;
        Ok(Self { spec })
    }
    pub fn spec(&self) -> &ReviewSpec {
        &self.spec
    }
    pub fn id(&self) -> OwnerBodyId {
        body_id(&self.encode())
    }
    pub fn case_id(&self) -> ReviewCaseId {
        ReviewCaseId(hash(CASE_DOMAIN, &self.spec.target.encode()))
    }
    /// Strict action binding only, with the exact same limitations as AcceptanceRecord:
    /// historical ordinary/dependency rights and original Profile/Admission need verification.
    pub fn matches_action(&self, action: &SignedAction) -> Result<(), ChatError> {
        self.spec.target.matches_action(action)
    }
    /// Signature/kind/group/Profile/body-CID binding only. Reject/Reopen are NEVER positive;
    /// even Approve needs selected-current-case/root/ordinary evidence before opaque issuance.
    pub fn verify_header_binding(&self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        check_header(
            header,
            HeaderKind::ReviewDecision,
            &self.spec.target.spec().group,
            self.spec.target.spec().context.profile,
            self.id(),
        )
    }
    /// Check a NEW case transition against the exact supplied previous body. This is neither
    /// an idempotency lookup nor proof that previous is the selected CURRENT case head. The
    /// active governor must reject a stale Reject after an approval and any initial rejection
    /// of a known already-finalized action. Approval never has a rejection/reopen successor.
    pub fn verify_new_transition_from(&self, previous: Option<&Self>) -> Result<(), ChatError> {
        match (previous, self.spec.previous) {
            (None, None) if self.spec.decision != ReviewDecision::Reopen => Ok(()),
            (Some(previous), Some(id))
                if previous.id() == id
                    && previous.case_id() == self.case_id()
                    && matches!(
                        (previous.spec.decision, self.spec.decision),
                        (ReviewDecision::Reject, ReviewDecision::Reopen)
                            | (
                                ReviewDecision::Reopen,
                                ReviewDecision::Approve | ReviewDecision::Reject
                            )
                    ) =>
            {
                Ok(())
            }
            _ => Err(ChatError::Dependency),
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_REVIEW_BODY_BYTES);
        e.put_bytes(DOMAIN).expect("fixed review domain");
        e.put_bytes(&self.spec.target.encode())
            .expect("bounded target record");
        e.put_bytes(&self.spec.intent).expect("fixed intent nonce");
        e.put_u8(self.spec.decision.tag())
            .put_u8(u8::from(self.spec.previous.is_some()));
        if let Some(previous) = self.spec.previous {
            e.put_bytes(previous.as_bytes())
                .expect("fixed previous decision");
        }
        e.finish()
    }
    /// Full OUTER borrowed framing/options/length validation precedes even the target decoder's
    /// owned group copy. There is no action-body copy, current-state lookup or signature claim.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_REVIEW_BODY_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != DOMAIN {
            return Err(ChatError::Malformed);
        }
        let target = d.get_bytes()?;
        if target.len() > MAX_ACCEPTANCE_RECORD_BYTES {
            return Err(ChatError::Bound);
        }
        let intent = d
            .get_bytes()?
            .try_into()
            .map_err(|_| ChatError::Malformed)?;
        let decision = ReviewDecision::from_tag(d.get_u8()?)?;
        let previous = match d.get_u8()? {
            0 => None,
            1 => Some(OwnerBodyId::from_bytes(digest(&mut d)?)),
            _ => return Err(ChatError::Malformed),
        };
        if decision == ReviewDecision::Reopen && previous.is_none() {
            return Err(ChatError::Dependency);
        }
        d.finish()?;
        Ok(Self {
            spec: ReviewSpec {
                target: AcceptanceRecord::decode(target)?,
                intent,
                decision,
                previous,
            },
        })
    }
}

#[cfg(test)]
mod tests;
